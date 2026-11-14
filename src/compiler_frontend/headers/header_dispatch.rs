//! Header declaration dispatch.
//!
//! WHAT: classifies one top-level declaration by the marker or syntax following its declaration name
//! and builds the concrete `HeaderKind` payload.
//! WHY: declaration-kind parsing is separate from per-file token walking and from dependency sorting.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, InvalidConfigReason, InvalidDeclarationReason,
};
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::datatypes::generic_parameters::GenericParameterList;
use crate::compiler_frontend::symbols::string_interning::StringId;

use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::choice::parse_choice_shell as parse_choice_header_payload;
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, parse_declaration_syntax,
};
use crate::compiler_frontend::declaration_syntax::generic_parameters::parse_generic_parameter_list_after_type_keyword;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    SignatureMemberSyntax, parse_function_signature_syntax_with_evaluation,
};

use crate::compiler_frontend::declaration_syntax::r#struct::parse_struct_shell;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    ParsedNamedTypeReference, ParsedParameterContractSyntax, TypeAnnotationContext,
    collect_capacity_references_in_parsed_ref, for_each_named_type_in_parsed_ref,
    parse_type_annotation_cursor,
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
use crate::compiler_frontend::tokenizer::line_scanning::{
    CloserPlacement, closer_layout_diagnostic_for_placement, closer_placement,
    has_physical_line_break_between,
};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenIndex, TokenRange, TokenRef, TokenTag,
};
use crate::compiler_frontend::traits::syntax::{
    ConformanceTargetKind, ConformanceTargetSyntax, TraitReferenceSyntax,
};
use crate::compiler_frontend::utilities::token_scan::{
    InitializerReference, NestingDepth, innermost_unclosed_delimiter,
    unclosed_delimiter_eof_diagnostic,
};
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
    /// Authored span of a pending `$config` source modifier, if one attaches to this declaration.
    pub(super) config_input_span: Option<SourceSpan>,
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

fn set_cursor_position(
    cursor: &mut TokenCursor<'_>,
    position: TokenIndex,
    owner: &'static str,
) -> HeaderDispatchResult<()> {
    cursor.set_position(position).map_err(|error| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            format!("{owner}: {error:?}",),
        ))
    })
}

fn cursor_span(cursor: &TokenCursor<'_>, fallback: SourceSpan) -> SourceSpan {
    cursor
        .current()
        .map_or(fallback, |token| token.source_span())
}
// Dispatches on the marker or syntax following the declaration name and delegates to kind-specific
// helpers where they exist, or captures body source ranges directly for simpler cases.
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
        config_input_span,
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
    let mut current_tag = source_tag_at_cursor(cursor, file_id)?;
    let next_tag = source_next_tag(cursor, file_id)?;
    let mut body_range = empty_token_range(cursor, file_id)?;

    // `#|` marks a const function. The `#` token itself carries no semantic payload, so consume
    // it and let the ordinary function arm parse the shared parameter and return grammar.
    let mut evaluation = CallableEvaluation::Runtime;
    if current_tag == TokenTag::HASH && next_tag == Some(TokenTag::TYPE_PARAMETER_BRACKET) {
        cursor.advance();
        current_tag = TokenTag::TYPE_PARAMETER_BRACKET;
        evaluation = CallableEvaluation::Const;
    }

    if current_tag == TokenTag::OF {
        if !generic_parameters.is_empty() {
            return Err(HeaderParseFailure::Diagnostic(
                names_declarations::invalid_declaration(
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
            source_directives: Vec::new(),
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
    //      declarations that participate in header parsing.
    if current_tag == TokenTag::MUST {
        if !generic_parameters.is_empty() {
            return Err(HeaderParseFailure::Diagnostic(
                names_declarations::invalid_declaration(
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
            // `next_tag` already established that `must` is followed by this colon.
            let body_opening_span =
                cursor
                    .peek_next()
                    .map(TokenRef::source_span)
                    .ok_or_else(|| {
                        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
                            "Trait declaration handoff did not retain its `must:` colon.",
                            Some(name_span),
                        ))
                    })?;
            let mut trait_cursor = *cursor;
            let _ = trait_cursor.advance(); // past must
            let _ = trait_cursor.advance(); // past :

            let declaration = parse_trait_declaration(
                &mut trait_cursor,
                name_span,
                declaration_name,
                body_opening_span,
                declaration_order,
                context,
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
                    collect_signature_member_ordering_hints(
                        param,
                        &generic_parameters,
                        full_name,
                        context,
                        &mut local_ordering_hints,
                        &mut capacity_references,
                    )?;
                }
                for ret in requirement.signature.ordinary_return_slots() {
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
            source_directives: Vec::new(),
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
                let mut declaration_cursor = DeclarationCursor::new(*cursor);
                let signature = parse_function_signature_syntax_with_evaluation(
                    &mut declaration_cursor,
                    context.warnings,
                    context.string_table,
                    full_name,
                    context.path_fork,
                    evaluation,
                )?;
                let cursor_position = declaration_cursor.canonical_cursor().position();
                (signature, cursor_position)
            };
            set_cursor_position(
                cursor,
                cursor_position,
                "function signature cursor handoff exceeded its source owner",
            )?;

            // Local declaration-ordering hints: parameter (including `Route` inputs) + return
            // type references only.
            for param in &signature.parameters {
                collect_signature_member_ordering_hints(
                    param,
                    &generic_parameters,
                    full_name,
                    context,
                    &mut local_ordering_hints,
                    &mut capacity_references,
                )?;
            }

            for ret in signature.ordinary_return_slots() {
                collect_type_ordering_hints(
                    &ret.value.type_annotation,
                    &generic_parameters,
                    full_name,
                    context,
                    &mut local_ordering_hints,
                    &mut capacity_references,
                )?;
            }

            // The signature parse exits immediately after its terminating `:`, which the body
            // capture reports when the file ends before the function's own `;`.
            let body_opening_span = cursor.parser_previous().map(TokenRef::source_span);
            let Some(body_opening_span) = body_opening_span else {
                return Err(HeaderParseFailure::Infrastructure(
                    internal_header_dispatch_error(
                        "Function signature handoff did not retain its terminating colon.",
                        Some(name_span),
                    ),
                ));
            };
            body_range = capture_function_body_range_from_cursor(
                cursor,
                file_id,
                body_opening_span,
                context.string_table,
            )?;

            kind = HeaderKind::Function {
                generic_parameters,
                signature,
            };
        }

        // `This` keyword outside trait declarations is invalid.
        TokenTag::TRAIT_THIS => {
            return Err(generics_traits_api::invalid_this_usage(crate::compiler_frontend::compiler_messages::InvalidThisUsageReason::OutsideTraitDeclaration,
            Some(cursor_span(cursor, declaration_span)),)
            .into());
        }

        // `=` only creates a declaration header for struct shells. Runtime top-level
        // `name = value` stays in the entry body outside `config.moth`.
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
                    let mut declaration_cursor = DeclarationCursor::new(*cursor);
                    let fields = parse_struct_shell(
                        &mut declaration_cursor,
                        context.string_table,
                        context.warnings,
                        full_name,
                        context.path_fork,
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
                config_input_span,
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
                let mut declaration_cursor = DeclarationCursor::new(*cursor);
                let choice_header = parse_choice_header_payload(
                    &mut declaration_cursor,
                    full_name,
                    context.path_fork,
                    context.string_table,
                    context.warnings,
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
                    names_declarations::invalid_declaration(
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
                let mut declaration_cursor = DeclarationCursor::new(*cursor);
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
                return Err(error);
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
        source_directives: Vec::new(),
        synthetic_content_payload: None,
        tokens: body_range,
        declaration_path: header_id,
        token_sequence: None,
        capacity_references,
    })
}

fn emit_header_naming_warning(
    warnings: &mut DiagnosticBag,
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
        let mut declaration_cursor = DeclarationCursor::new(*cursor);
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
) -> Result<(), HeaderParseFailure> {
    let mut selection_error = None;
    for_each_named_type_in_parsed_ref(type_ref, &mut |type_reference| {
        if selection_error.is_some() {
            return;
        }

        match type_reference {
            // Generic parameters are bare local names. A qualified path must remain intact even
            // when its terminal component happens to share a generic parameter's spelling.
            ParsedNamedTypeReference::Bare {
                name: type_name, ..
            } => {
                if generic_parameters.contains_name(type_name)
                    || context.path_fork.component(current_header_path) == Some(type_name)
                {
                    return;
                }
            }
            ParsedNamedTypeReference::Qualified { .. } => {}
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

/// Collect ordering hints for one signature member.
///
/// WHAT: walks the member's ordinary type reference and any type syntax inside a `Route`
///       contract's input.
/// WHY: a signature that names a type only as a `Route` input still depends on that type's
///      declaration, so ordering must see the same type syntax resolution later resolves.
fn collect_signature_member_ordering_hints(
    member: &SignatureMemberSyntax,
    generic_parameters: &GenericParameterList,
    current_header_path: PathId,
    context: &mut HeaderBuildContext<'_>,
    local_ordering_hints: &mut HashSet<LocalDeclarationOrderingHint>,
    capacity_references: &mut Vec<InitializerReference>,
) -> HeaderDispatchResult<()> {
    collect_type_ordering_hints(
        &member.type_annotation,
        generic_parameters,
        current_header_path,
        context,
        local_ordering_hints,
        capacity_references,
    )?;
    if let ParsedParameterContractSyntax::Route {
        input: Some(input), ..
    } = &member.contract
    {
        collect_type_ordering_hints(
            input,
            generic_parameters,
            current_header_path,
            context,
            local_ordering_hints,
            capacity_references,
        )?;
    }
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

/// Whether a newline after `tag` continues the same expression.
///
/// WHAT: mirrors the expression parser's logical-line rule for the tokens the body scanner sees.
/// WHY: a region header starts a fresh statement, while `if count >` followed by `limit:` on the
/// next line is one header. The distinction keeps a continued header from adding a phantom scope.
fn tag_continues_into_next_line(tag: TokenTag) -> bool {
    tag.continues_expression() && !matches!(tag, TokenTag::END | TokenTag::TYPE_PARAMETER_BRACKET)
}

/// Record a terminal branch continuation on the innermost open source scope.
///
/// WHAT: records the canonical index of a top-level `else` (for an `if`) or `=>` (for a match).
/// WHY: the accepted contract lets the final `;` of those branches share the branch's physical
///      line. The checker must compare that continuation token with the closer; an earlier
///      continuation cannot excuse a closer on a different physical line.
fn mark_branch_continuation(
    open_scopes: &mut [OpenScope],
    top_level: bool,
    continuation: TokenIndex,
) {
    if top_level && let Some(scope) = open_scopes.last_mut() {
        scope.continuation_index = Some(continuation);
    }
}

/// One source scope still open inside the captured function body.
///
/// WHY: the capture walk must keep each opener's canonical index so a closer it balances can be
///      classified on the same token store. `continuation_index` is the latest top-level `else`
///      or `=>` at this scope's level; only a closer that shares its physical line may use it as
///      its effective opening site.
struct OpenScope {
    opening_index: TokenIndex,
    opening_span: SourceSpan,
    continuation_index: Option<TokenIndex>,
}

/// Classify an inner closer, applying the branch-continuation exception only when it is on the
/// closer's physical line.
///
/// WHAT: uses the latest top-level `else`/`=>` as the effective opening site when no physical
///       line break separates it from the closer; otherwise uses the scope's own opener.
/// WHY: `else return;;` is legal because the closer shares the `else` line, while an `else` on an
///      earlier line does not excuse a closer after code.
fn inner_closer_placement(
    source: &SourceTokens,
    scope: &OpenScope,
    closer: TokenIndex,
) -> Option<CloserPlacement> {
    let opening = scope
        .continuation_index
        .filter(|continuation| !has_physical_line_break_between(source, *continuation, closer))
        .unwrap_or(scope.opening_index);
    closer_placement(source, opening, closer)
}

/// Capture a function body directly from a canonical cursor.
///
/// WHAT: consumes the cursor that the declaration parser left on the first body token and returns
/// the source range from that token through its matching `;`, tracking the still-open scopes so
/// nested scopes (inner `if`/`loop`/etc.) close correctly.
/// WHY: the canonical header path must retain ranges, not clone a token vector or
/// rescan source text, and the cursor is the sole source of parser position and EOF/span facts.
/// Only the structural range is captured here: AST owns body syntax diagnostics, declaration
/// ordering hints come from the signature, and Stage 0 records graph-active file references from
/// its own source-owned start-body ranges.
///
/// `body_opening_span` is the `:` that ended the signature. It labels the missing closer when the
/// file ends before the function body closes.
pub(super) fn capture_function_body_range_from_cursor(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    body_opening_span: SourceSpan,
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
    // The signature parser consumes the `:`. Deriving its canonical index from the first body
    // token keeps the closer's placement check on the same token store without retaining a
    // second position: the `:` is always the token immediately before the first body token.
    let body_opening_index = body_start
        .index()
        .checked_sub(1)
        .and_then(TokenIndex::try_from_index)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
                "Function body capture started before its opening colon.",
                Some(body_opening_span),
            ))
        })?;
    let opening_token = source.token(body_opening_index).map_err(|_| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Function body capture could not read its opening colon.",
            Some(body_opening_span),
        ))
    })?;
    if opening_token.tag() != TokenTag::COLON {
        return Err(HeaderParseFailure::Infrastructure(
            internal_header_dispatch_error(
                "Function signature handoff did not leave its opening colon before the body.",
                Some(body_opening_span),
            ),
        ));
    }
    // One entry per still-open scope, outermost first. The function body's own colon is the base
    // entry, so the body closes exactly when the stack empties. Keeping each opener's index and
    // span here is what lets the end-of-file report name the block that never closed and lets a
    // stacked closer report the inner scope that was closed first.
    let mut open_scopes = vec![OpenScope {
        opening_index: body_opening_index,
        opening_span: body_opening_span,
        continuation_index: None,
    }];
    // The most recent top-level `;` that closed a nested block. When only the function body is
    // left open at end of file, that `;` is the one a reader would expect to have closed the
    // function, so the report can point at it.
    let mut last_inner_closer_span: Option<SourceSpan> = None;
    // The layout error of the first inner closer that is misplaced or followed by code on its
    // line. Reported only when the function closer is also invalid, so the author sees the first
    // structural error in the file. It is ordering evidence only: it never decides acceptance.
    let mut earliest_invalid_inner: Option<DiagnosticDraft> = None;
    // A `;` seen inside `(...)`, `{...}` or a template already names an earlier AST error. The
    // function-closer layout check must stay silent so the source-accurate grouped diagnostic wins.
    let mut saw_grouped_closer = false;
    let mut body_end = None;
    // The authored span of the function body's own `;`, paired with `body_end` so the layout
    // check can report at the closer the author wrote.
    let mut body_closer_span: Option<SourceSpan> = None;
    let mut nesting = NestingDepth::default();
    // A block opener is the colon that ends an `if`, `catch` or `loop` header. Colons in
    // malformed match-arm headers (`1:` or `else:`) belong to the arm syntax and must not open a
    // scope here, or this stage reports EOF before the AST can name the missing `=>`. The flag
    // survives newlines because a condition may continue after an expression-continuing token,
    // and it clears only when its colon is consumed or the header resolves inline through `then`.
    let mut block_header_pending = false;
    // The scope depth of the outermost open match body. A match header ends `is:` while other
    // block headers keep a condition or binding before their colon. Its arms use `name =>`, so a
    // line-initial `name:` inside one is a malformed arm that the AST must name, and it cannot
    // declare a region there. Suppression stays on at or above that depth, so an arm-shaped colon
    // cannot invent a scope that the function's real closing `;` never balances. Closing the
    // match body clears the floor.
    let mut match_body_floor: Option<usize> = None;
    // Declared-region headers (`name:`) look like a symbol followed by a colon at the start of a
    // line. Tracking line starts keeps match-arm and field colons out of that shape, and a line
    // that continues the previous expression (`if count >` then `limit:`) is not a region.
    let mut at_line_start = true;
    let mut previous_meaningful_tag = None;

    // `parse_function_signature_syntax` stops on the first body token, so inspect the current
    // canonical token before moving the cursor. EOF is handled from its exact TokenRef span.
    while !open_scopes.is_empty() {
        let current = cursor.advance().ok_or_else(|| {
            HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
                "Function body capture reached the source end before its closing delimiter.",
                None,
            ))
        })?;

        match current.tag() {
            // A `;` closes the active block only at top-level nesting. Inside `(...)`, `{...}` or
            // a template head it is not a block closer, so it stays in the captured body range for
            // the AST to report at its own span and the cursor keeps reading the rest of the body.
            TokenTag::END if !nesting.is_top_level() => {
                block_header_pending = false;
                saw_grouped_closer = true;
            }
            TokenTag::END => {
                block_header_pending = false;
                let closed_scope = open_scopes.pop();
                if match_body_floor.is_some_and(|floor| open_scopes.len() < floor) {
                    match_body_floor = None;
                }
                if open_scopes.is_empty() {
                    body_end = Some(current.index());
                    body_closer_span = Some(current.source_span());
                } else if let Some(scope) = closed_scope {
                    last_inner_closer_span = Some(current.source_span());
                    if earliest_invalid_inner.is_none() {
                        let placement = inner_closer_placement(source, &scope, current.index());
                        earliest_invalid_inner = closer_layout_diagnostic_for_placement(
                            source,
                            placement,
                            scope.opening_span,
                            current.index(),
                            current.source_span(),
                        );
                    }
                }
            }
            // A block header only opens a source scope when the header itself sits at top-level
            // nesting. A header-shaped keyword inside `(...)`, `{...}` or a template head is
            // misplaced syntax that the AST names, and leaving a pending colon armed there would
            // let it open a phantom scope on a later top-level colon.
            TokenTag::IF | TokenTag::CATCH | TokenTag::LOOP => {
                block_header_pending = nesting.is_top_level();
            }
            // Inline headers resolve through `then`, so they open no source scope.
            TokenTag::THEN => block_header_pending = false,
            // A match guard resolves through `=>`. The match body stays open, but its terminal
            // arm may legally share its line with the closing `;`, so record that continuation.
            TokenTag::FAT_ARROW => {
                block_header_pending = false;
                // A `=>` directly after a closer cannot continue an arm of the scope that is
                // still open; recording it would invent an exception for the innermost scope.
                if previous_meaningful_tag != Some(TokenTag::END) {
                    mark_branch_continuation(
                        &mut open_scopes,
                        nesting.is_top_level(),
                        current.index(),
                    );
                }
            }
            // An `else` introducing the terminal `if` body may legally share its line with the
            // final `;`; the scope that stays open must remember that continuation token.
            TokenTag::ELSE => {
                if previous_meaningful_tag != Some(TokenTag::END) {
                    mark_branch_continuation(
                        &mut open_scopes,
                        nesting.is_top_level(),
                        current.index(),
                    );
                }
            }
            // Colons used in templates parse into a different token (`StartTemplateBody`), so
            // only a block-header colon still contributes one balanced source scope.
            TokenTag::COLON if block_header_pending => {
                block_header_pending = false;
                // Only a top-level colon opens a block. A colon inside a group belongs to
                // malformed header syntax and must not unbalance the captured function body.
                if nesting.is_top_level() {
                    open_scopes.push(OpenScope {
                        opening_index: current.index(),
                        opening_span: current.source_span(),
                        continuation_index: None,
                    });
                    if previous_meaningful_tag == Some(TokenTag::IS) {
                        let depth = open_scopes.len();
                        match_body_floor =
                            Some(match_body_floor.map_or(depth, |floor| floor.min(depth)));
                    }
                }
            }
            // A line-initial `name:` declares a region, which balances its `;` like a block.
            TokenTag::SYMBOL
                if at_line_start
                    && nesting.is_top_level()
                    && match_body_floor.is_none()
                    && !previous_meaningful_tag.is_some_and(tag_continues_into_next_line)
                    && cursor
                        .peek()
                        .is_some_and(|token| token.tag() == TokenTag::COLON) =>
            {
                let region_opening = cursor.peek().map_or_else(
                    || OpenScope {
                        opening_index: current.index(),
                        opening_span: current.source_span(),
                        continuation_index: None,
                    },
                    |token| OpenScope {
                        opening_index: token.index(),
                        opening_span: token.source_span(),
                        continuation_index: None,
                    },
                );
                open_scopes.push(region_opening);
            }
            // An arm-header colon or `else:` inside a match block stays part of that block.
            TokenTag::COLON => {}
            TokenTag::DOUBLE_COLON => {}
            TokenTag::EOF => {
                let unclosed = innermost_unclosed_delimiter(source, body_start, current.index());
                let diagnostic = unclosed_delimiter_eof_diagnostic(
                    unclosed,
                    Some(current.source_span()),
                    Some(";"),
                    string_table,
                );
                // A still-open `(`/`{`/template already names its own opener. Otherwise the block
                // that never closed owns the report.
                let diagnostic = if unclosed.is_some() {
                    diagnostic
                } else {
                    add_function_body_eof_labels(&open_scopes, last_inner_closer_span, diagnostic)
                };
                return Err(diagnostic.into());
            }
            _ => {}
        }
        at_line_start = current.tag() == TokenTag::NEWLINE;
        if current.tag() != TokenTag::NEWLINE {
            previous_meaningful_tag = Some(current.tag());
        }
        nesting.step_tag(current.tag());
    }

    let body_end = body_end.ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Function body capture did not identify its closing delimiter.",
            None,
        ))
    })?;
    let body_closer_span = body_closer_span.ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Function body capture lost its closing delimiter span.",
            Some(body_opening_span),
        ))
    })?;
    // Stage 2 owns this closer: the retained body range is half-open at it, so AST never sees
    // the token. Check its placement and its line's remainder before the range escapes.
    if !saw_grouped_closer {
        let function_placement = closer_placement(source, body_opening_index, body_end);
        let function_diagnostic = closer_layout_diagnostic_for_placement(
            source,
            function_placement,
            body_opening_span,
            body_end,
            body_closer_span,
        );
        // When the function closer is itself invalid, report the earliest invalid inner closer
        // instead: it is the first structural error in the file. The header error ends this
        // file's compilation, so AST never runs to report that inner closer, and the
        // classification above is only evidence for this ordering decision. If the function
        // closer is valid, inner closers stay AST's responsibility.
        if function_diagnostic.is_some()
            && let Some(diagnostic) = earliest_invalid_inner
        {
            return Err(HeaderParseFailure::Diagnostic(diagnostic));
        }
        if let Some(diagnostic) = function_diagnostic {
            return Err(HeaderParseFailure::Diagnostic(diagnostic));
        }
    }
    TokenRange::try_new_for(source, body_start, body_end).map_err(|error| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            format!("function body token range was invalid: {error:?}"),
        ))
    })
}

/// Attach the block that never closed, and any stolen closer, to a function-body EOF report.
///
/// WHAT: labels the innermost still-open block's opening colon. When only the function body
/// remains open, also labels the last top-level `;` that closed a nested block.
/// WHY: "expected ';'" at the end of file does not say which block is open. A reader who wrote a
/// `;` needs to see that it closed an inner block, leaving the function unclosed.
fn add_function_body_eof_labels(
    open_scopes: &[OpenScope],
    last_inner_closer: Option<SourceSpan>,
    diagnostic: DiagnosticDraft,
) -> DiagnosticDraft {
    let mut diagnostic = diagnostic;
    if let Some(innermost_open) = open_scopes.last() {
        diagnostic = diagnostic.with_label(
            Some(innermost_open.opening_span),
            Some(LabelDraftMessage::UnclosedBlockStartsHere),
        );
    }
    // The stolen-closer explanation is only exact when the function body itself is the single
    // remaining open scope: every consumed `;` then closed a nested block.
    if open_scopes.len() == 1
        && let Some(closer) = last_inner_closer
    {
        diagnostic = diagnostic.with_label(Some(closer), Some(LabelDraftMessage::InnerBlockCloser));
    }
    diagnostic
}
#[allow(
    clippy::too_many_arguments,
    reason = "the constant shell needs the shared cursor, context and two collection outputs \
              beside the authored spans"
)]
fn create_constant_header_payload(
    full_name: &PathId,
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    context: &mut HeaderBuildContext<'_>,
    local_ordering_hints: &mut HashSet<LocalDeclarationOrderingHint>,
    capacity_references: &mut Vec<InitializerReference>,
    span_builder: &mut ExtendedSpanBuilder,
    config_input_span: Option<SourceSpan>,
) -> HeaderDispatchResult<DeclarationSyntax> {
    let Some(declaration_name) = context.path_fork.component(*full_name) else {
        return Err(internal_header_dispatch_error(
            "Constant header path is missing its declaration name.",
            cursor.current().map(|token| token.source_span()),
        )
        .into());
    };
    let (declaration_syntax, cursor_position) = {
        let mut declaration_cursor = DeclarationCursor::new(*cursor);
        let declaration_syntax = parse_declaration_syntax(
            &mut declaration_cursor,
            declaration_name,
            context.string_table,
            span_builder,
            config_input_span,
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
        && let Some(contract) = &declaration_syntax.config_contract
    {
        return Err(project_config::invalid_config_reason(
            Some(declaration_name),
            InvalidConfigReason::ConfigQualifierInvalidPlacement,
            Some(contract.marker_span),
        )
        .into());
    }

    // Local declaration-ordering hints: declared type annotation only.
    // WHY: constant initializer reference edges are collected separately in
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
    _span: Option<SourceSpan>,
) -> InfrastructureFailure {
    InfrastructureFailure::internal(CompilerStage::HeaderParsing, message)
}
