//! Header declaration dispatch.
//!
//! WHAT: classifies one top-level declaration after its leading symbol has been seen and builds the
//! concrete `HeaderKind` payload.
//! WHY: declaration-kind parsing is separate from per-file token walking and from dependency sorting.

use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidDeclarationReason};
use crate::compiler_frontend::datatypes::generic_parameters::GenericParameterList;
use crate::compiler_frontend::symbols::string_interning::StringId;

use crate::compiler_frontend::declaration_syntax::choice::parse_choice_shell as parse_choice_header_payload;
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, parse_declaration_syntax,
};
use crate::compiler_frontend::declaration_syntax::generic_parameters::parse_generic_parameter_list_after_type_keyword;
use crate::compiler_frontend::declaration_syntax::signature_members::parse_function_signature_syntax;

use crate::compiler_frontend::declaration_syntax::r#struct::parse_struct_shell;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, collect_capacity_references_in_parsed_ref,
    for_each_named_type_in_parsed_ref, parse_type_annotation,
};

use super::trait_headers::{
    conformance_header_path, ensure_trait_name_is_all_caps, incompatibility_header_path,
    parse_specialized_conformance_target, parse_trait_conformance, parse_trait_declaration,
    parse_trait_incompatibility,
};
use crate::compiler_frontend::headers::ordering_hints::{
    collect_constant_type_hints, collect_named_type_ordering_hint,
};
use crate::compiler_frontend::headers::types::{
    Header, HeaderBuildContext, HeaderExportMode, HeaderKind, LocalDeclarationOrderingHint,
};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::traits::syntax::{
    ConformanceTargetKind, ConformanceTargetSyntax, TraitReferenceSyntax,
};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use rustc_hash::FxHashSet;
use std::collections::HashSet;

/// Boxed diagnostic result for header dispatch.
///
/// WHAT: gives declaration dispatch and its local helpers one small error boundary.
/// WHY: delegated declaration parsers already return boxed diagnostics, so dispatch can
///      propagate them directly without unboxing and reboxing between each step.
type HeaderDispatchResult<T> = Result<T, Box<CompilerDiagnostic>>;

// WHAT: classifies one top-level declaration by its leading token and builds the concrete header
// payload (kind + body token slice + dependency set) that later AST passes consume.
//
// WHY: every declaration kind (function, struct, choice/union, constant) has a different leading
// token pattern. This function dispatches on that token and delegates to kind-specific helpers
// where they exist, or captures body tokens directly for simpler cases.
//
// Dispatch summary:
//   `|`  (TypeParameterBracket)  → function signature + body token capture
//   `=`  (Assign)                → struct `= |fields|`
//   `::`  (DoubleColon)          → choice/union variant list
//   `#`  (Hash)                  → compile-time constant binding `#=` / `#Type`
//   `must:`                      → trait declaration shell
//   `must TRAIT`                 → trait conformance shell
//   `This`                       → trait-local keyword outside a trait declaration, error
//   anything else                → no header created (e.g. start-template body lines)
pub(super) fn create_header(
    full_name: InternedPath,
    token_stream: &mut FileTokens,
    name_location: SourceLocation,
    export_mode: HeaderExportMode,
    context: &mut HeaderBuildContext<'_>,
) -> HeaderDispatchResult<Header> {
    let Some(declaration_name) = full_name.name() else {
        return Err(internal_header_dispatch_error(
            "Header declaration path is missing its declaration name.",
            name_location,
        )
        .into());
    };
    let _declaration_name_text = context.string_table.resolve(declaration_name).to_owned();

    // Conservative local declaration-ordering hints; binding and Stage 3 resolve them.
    let mut local_ordering_hints: HashSet<LocalDeclarationOrderingHint> = HashSet::new();
    let mut kind: HeaderKind = HeaderKind::StartFunction;
    let mut body = Vec::new();
    let mut capacity_references: Vec<InitializerReference> = Vec::new();
    let generic_parameters = parse_optional_generic_parameters(token_stream, context)?;

    if token_stream.current_token_kind() == &TokenKind::Of {
        if !generic_parameters.is_empty() {
            return Err(Box::new(CompilerDiagnostic::invalid_declaration(
                InvalidDeclarationReason::GenericTraitsUnsupported,
                Some(declaration_name),
                name_location,
            )));
        }

        let target = parse_specialized_conformance_target(
            token_stream,
            declaration_name,
            name_location.clone(),
        )?;
        token_stream.advance(); // past must

        let conformance = parse_trait_conformance(token_stream, target, context)?;
        kind = HeaderKind::TraitConformance { conformance };

        let conformance_path =
            conformance_header_path(&full_name, &name_location, context.string_table);
        let mut header_tokens =
            FileTokens::new_with_file_id(conformance_path, token_stream.file_id, body);
        header_tokens.canonical_os_path = token_stream.canonical_os_path.clone();

        return Ok(Header {
            kind,
            file_role: context.file_role,
            export_mode,
            local_ordering_hints,
            name_location,
            tokens: header_tokens,
            source_file: context.source_file.to_owned(),
            capacity_references,
        });
    }

    // Check for trait syntax after generic parameters and before token dispatch.
    // WHAT: `must:` begins a trait declaration; `must not` begins an incompatibility
    //      declaration; `must TRAIT` begins a conformance declaration.
    // WHY: trait declarations, conformances, and incompatibility metadata are top-level
    //      declarations that participate in header parsing; they replace the old
    //      reserved-trait rejection path.
    if token_stream.current_token_kind() == &TokenKind::Must {
        if !generic_parameters.is_empty() {
            return Err(Box::new(CompilerDiagnostic::invalid_declaration(
                InvalidDeclarationReason::GenericTraitsUnsupported,
                Some(declaration_name),
                name_location,
            )));
        }

        let peek = token_stream.peek_next_token().cloned();

        if peek == Some(TokenKind::Not) {
            ensure_trait_name_is_all_caps(
                declaration_name,
                name_location.clone(),
                context.string_table,
            )?;

            // Trait incompatibility declaration: `Name must not TRAIT, TRAIT`
            token_stream.advance(); // past must
            token_stream.advance(); // past not

            let subject = TraitReferenceSyntax {
                name: declaration_name,
                location: name_location.clone(),
            };
            let incompatibility = parse_trait_incompatibility(token_stream, subject, context)?;
            kind = HeaderKind::TraitIncompatibility { incompatibility };
        } else if peek == Some(TokenKind::Colon) {
            ensure_trait_name_is_all_caps(
                declaration_name,
                name_location.clone(),
                context.string_table,
            )?;

            // Trait declaration: `Name must: requirements ;`
            token_stream.advance(); // past must
            token_stream.advance(); // past :

            let declaration = parse_trait_declaration(
                token_stream,
                declaration_name,
                name_location.clone(),
                context,
            )?;

            // Collect local declaration-ordering hints from requirement signatures.
            for requirement in &declaration.requirements {
                for param in &requirement.signature.parameters {
                    collect_type_ordering_hints(
                        &param.type_annotation,
                        &generic_parameters,
                        &full_name,
                        context,
                        &mut local_ordering_hints,
                        &mut capacity_references,
                    );
                }
                for ret in &requirement.signature.returns {
                    collect_type_ordering_hints(
                        &ret.value.type_annotation,
                        &generic_parameters,
                        &full_name,
                        context,
                        &mut local_ordering_hints,
                        &mut capacity_references,
                    );
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
                    location: name_location.clone(),
                },
                context,
            )?;

            kind = HeaderKind::TraitConformance { conformance };
        }

        let header_path = match kind {
            HeaderKind::TraitConformance { .. } => {
                conformance_header_path(&full_name, &name_location, context.string_table)
            }
            HeaderKind::TraitIncompatibility { .. } => {
                incompatibility_header_path(&full_name, &name_location, context.string_table)
            }
            _ => full_name,
        };
        let mut header_tokens =
            FileTokens::new_with_file_id(header_path, token_stream.file_id, body);
        header_tokens.canonical_os_path = token_stream.canonical_os_path.clone();

        return Ok(Header {
            kind,
            file_role: context.file_role,
            export_mode,
            local_ordering_hints,
            name_location,
            tokens: header_tokens,
            source_file: context.source_file.to_owned(),
            capacity_references,
        });
    }

    let current_token = token_stream.current_token_kind().to_owned();

    match current_token {
        // Function declaration: `name |params| -> return_type : body ;`
        TokenKind::TypeParameterBracket => {
            ensure_not_keyword_shadow_identifier(
                declaration_name,
                name_location.to_owned(),
                context.string_table,
            )?;
            emit_header_naming_warning(
                context.warnings,
                declaration_name,
                name_location.to_owned(),
                IdentifierNamingKind::ValueLike,
                context.string_table,
            );

            let signature = parse_function_signature_syntax(
                token_stream,
                context.warnings,
                context.string_table,
                &full_name,
            )?;

            // Local declaration-ordering hints: parameter + return type references only.
            for param in &signature.parameters {
                collect_type_ordering_hints(
                    &param.type_annotation,
                    &generic_parameters,
                    &full_name,
                    context,
                    &mut local_ordering_hints,
                    &mut capacity_references,
                );
            }

            for ret in &signature.returns {
                collect_type_ordering_hints(
                    &ret.value.type_annotation,
                    &generic_parameters,
                    &full_name,
                    context,
                    &mut local_ordering_hints,
                    &mut capacity_references,
                );
            }

            capture_function_body_tokens(token_stream, &mut body, context.string_table)?;

            kind = HeaderKind::Function {
                generic_parameters,
                signature,
            };
        }

        // `This` keyword outside trait declarations is invalid.
        TokenKind::TraitThis => {
            return Err(CompilerDiagnostic::invalid_this_usage(
                crate::compiler_frontend::compiler_messages::InvalidThisUsageReason::OutsideTraitDeclaration,
                token_stream.current_location(),
            )
            .into());
        }

        // `=` only creates a declaration header for struct shells. Runtime top-level
        // `name = value` stays in the entry start body outside `config.moth`.
        TokenKind::Assign => {
            if let Some(TokenKind::TypeParameterBracket) = token_stream.peek_next_token() {
                ensure_not_keyword_shadow_identifier(
                    declaration_name,
                    name_location.to_owned(),
                    context.string_table,
                )?;
                emit_header_naming_warning(
                    context.warnings,
                    declaration_name,
                    name_location.to_owned(),
                    IdentifierNamingKind::TypeLike,
                    context.string_table,
                );

                token_stream.advance();

                // Parse field shell directly — avoids reparsing in the AST type-resolution pass.
                // WHY: the header stage owns top-level shell parsing; AST owns body/executable parsing.
                let fields = parse_struct_shell(
                    token_stream,
                    context.string_table,
                    context.warnings,
                    &full_name,
                )?;

                // Collect strict type edges from field types only (no default-expression edges).
                // WHY: struct field type refs are the only struct edges that constrain sort order.
                for field in &fields {
                    collect_type_ordering_hints(
                        &field.type_annotation,
                        &generic_parameters,
                        &full_name,
                        context,
                        &mut local_ordering_hints,
                        &mut capacity_references,
                    );
                }

                kind = HeaderKind::Struct {
                    generic_parameters,
                    fields,
                };
            }
        }

        // `#` (Hash): compile-time constant declaration `name #= value` or `name #Type = value`.
        TokenKind::Hash => {
            ensure_not_keyword_shadow_identifier(
                declaration_name,
                name_location.to_owned(),
                context.string_table,
            )?;
            emit_header_naming_warning(
                context.warnings,
                declaration_name,
                name_location.to_owned(),
                IdentifierNamingKind::TopLevelConstant,
                context.string_table,
            );

            let constant_header = create_constant_header_payload(
                &full_name,
                token_stream,
                context,
                &mut local_ordering_hints,
                &mut capacity_references,
            )?;

            kind = HeaderKind::Constant {
                declaration: constant_header,
            };
        }

        // `::` (DoubleColon): choice/union declaration `name :: VariantA | VariantB | ...`
        TokenKind::DoubleColon => {
            ensure_not_keyword_shadow_identifier(
                declaration_name,
                name_location.to_owned(),
                context.string_table,
            )?;
            emit_header_naming_warning(
                context.warnings,
                declaration_name,
                name_location.to_owned(),
                IdentifierNamingKind::TypeLike,
                context.string_table,
            );

            let choice_header = parse_choice_header_payload(
                token_stream,
                &full_name,
                context.string_table,
                context.warnings,
            )
            .map_err(CompilerDiagnostic::from)?;

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
                            &full_name,
                            context,
                            &mut local_ordering_hints,
                            &mut capacity_references,
                        );
                    }
                }
            }

            kind = HeaderKind::Choice {
                generic_parameters,
                variants: choice_header,
            };
        }

        // `as`: type alias declaration `Name as Type`
        TokenKind::As => {
            if !generic_parameters.is_empty() {
                return Err(Box::new(CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::ParameterizedGenericTypeAlias,
                    Some(declaration_name),
                    name_location.to_owned(),
                )));
            }

            ensure_not_keyword_shadow_identifier(
                declaration_name,
                name_location.to_owned(),
                context.string_table,
            )?;
            emit_header_naming_warning(
                context.warnings,
                declaration_name,
                name_location.to_owned(),
                IdentifierNamingKind::TypeLike,
                context.string_table,
            );

            token_stream.advance();
            let target = parse_type_annotation(
                token_stream,
                TypeAnnotationContext::TypeAliasTarget,
                context.string_table,
            )?;

            for_each_named_type_in_parsed_ref(&target, &mut |type_name| {
                collect_named_type_ordering_hint(
                    type_name,
                    context.file_import_entries,
                    context.source_file,
                    context.string_table,
                    &mut local_ordering_hints,
                );
            });
            collect_capacity_references_in_parsed_ref(&target, &mut capacity_references);

            kind = HeaderKind::TypeAlias { target };
        }

        _ => {}
    }

    let mut header_tokens = FileTokens::new_with_file_id(full_name, token_stream.file_id, body);
    header_tokens.canonical_os_path = token_stream.canonical_os_path.clone();

    Ok(Header {
        kind,
        file_role: context.file_role,
        export_mode,
        local_ordering_hints,
        name_location,
        tokens: header_tokens,
        source_file: context.source_file.to_owned(),
        capacity_references,
    })
}

fn emit_header_naming_warning(
    warnings: &mut Vec<CompilerDiagnostic>,
    identifier: StringId,
    location: SourceLocation,
    naming_kind: IdentifierNamingKind,
    string_table: &crate::compiler_frontend::symbols::string_interning::StringTable,
) {
    if let Some(warning) =
        naming_warning_for_identifier(identifier, location, naming_kind, string_table)
    {
        warnings.push(warning);
    }
}

fn parse_optional_generic_parameters(
    token_stream: &mut FileTokens,
    context: &mut HeaderBuildContext<'_>,
) -> HeaderDispatchResult<GenericParameterList> {
    if token_stream.current_token_kind() != &TokenKind::Type {
        return Ok(GenericParameterList::default());
    }

    let forbidden_names = generic_parameter_forbidden_names(context);
    parse_generic_parameter_list_after_type_keyword(
        token_stream,
        &forbidden_names,
        context.string_table,
    )
}

fn generic_parameter_forbidden_names(context: &HeaderBuildContext<'_>) -> FxHashSet<StringId> {
    // WHAT: local names already claimed by retained import shells in this file.
    // WHY: import aliases are retained syntax, so this collision is provider-independent and
    // belongs in syntax preparation. Prelude type-symbol collisions are provider-dependent and
    // are validated during header binding against the bound prelude visibility instead.
    let mut forbidden_names = FxHashSet::default();

    for import in context.file_import_entries {
        if let Some(local_name) = import.alias.or_else(|| import.provider.path.name()) {
            forbidden_names.insert(local_name);
        }
    }

    forbidden_names
}

fn collect_type_ordering_hints(
    type_ref: &crate::compiler_frontend::datatypes::parsed::ParsedTypeRef,
    generic_parameters: &GenericParameterList,
    current_header_path: &InternedPath,
    context: &mut HeaderBuildContext<'_>,
    local_ordering_hints: &mut HashSet<LocalDeclarationOrderingHint>,
    capacity_references: &mut Vec<InitializerReference>,
) {
    for_each_named_type_in_parsed_ref(type_ref, &mut |type_name| {
        if generic_parameters.contains_name(type_name) {
            return;
        }

        if context.source_file.append(type_name) == *current_header_path {
            return;
        }

        collect_named_type_ordering_hint(
            type_name,
            context.file_import_entries,
            context.source_file,
            context.string_table,
            local_ordering_hints,
        );
    });
    collect_capacity_references_in_parsed_ref(type_ref, capacity_references);
}

// WHAT: collects all tokens that make up a function body (`:` … `;`) into `body`,
// tracking scope depth to handle nested scopes (inner `if`/`loop`/etc.) correctly.
//
// WHY: extracted from `create_header` to reduce its length and make the scope-balancing
// contract explicit. The token stream must already be positioned on the first body token
// (i.e. `FunctionSignature::new` has already consumed the signature).
// Local declaration-ordering hints are derived from the signature only; body tokens are captured but
// not scanned for imports — that is AST's responsibility at body-lowering time.
fn capture_function_body_tokens(
    token_stream: &mut FileTokens,
    body: &mut Vec<crate::compiler_frontend::tokenizer::tokens::Token>,
    string_table: &mut StringTable,
) -> HeaderDispatchResult<()> {
    let mut scopes_opened = 1;
    let mut scopes_closed = 0;

    // `FunctionSignature::new` stops on the first body token, so the first loop
    // iteration must inspect the current token before advancing.
    while scopes_opened > scopes_closed {
        match token_stream.current_token_kind() {
            TokenKind::End => {
                scopes_closed += 1;
                if scopes_opened > scopes_closed {
                    body.push(token_stream.current_token());
                }
            }

            // Colons used in templates parse into a different token (StartTemplateBody),
            // so there is no risk of templates creating a colon imbalance here.
            // All other language constructs follow the invariant: every `:` is closed by `;`.
            TokenKind::Colon => {
                scopes_opened += 1;
                body.push(token_stream.current_token());
            }

            // `::` is an expression/operator token (e.g. `Choice::Variant`) and must not
            // affect function-scope depth balancing.
            TokenKind::DoubleColon => {
                body.push(token_stream.current_token());
            }

            TokenKind::Eof => {
                // Diagnostic payloads carry the expected delimiter as a StringId so they can be
                // remapped and rendered through the active string table.
                return Err(CompilerDiagnostic::unexpected_end_of_file(
                    Some(string_table.intern(";")),
                    token_stream.current_location(),
                )
                .into());
            }

            _ => {
                body.push(token_stream.current_token());
            }
        }

        token_stream.advance();
    }

    Ok(())
}

fn create_constant_header_payload(
    full_name: &InternedPath,
    token_stream: &mut FileTokens,
    context: &mut HeaderBuildContext<'_>,
    local_ordering_hints: &mut HashSet<LocalDeclarationOrderingHint>,
    capacity_references: &mut Vec<InitializerReference>,
) -> HeaderDispatchResult<DeclarationSyntax> {
    let Some(declaration_name) = full_name.name() else {
        return Err(internal_header_dispatch_error(
            "Constant header path is missing its declaration name.",
            token_stream.current_location(),
        )
        .into());
    };
    let declaration_syntax =
        parse_declaration_syntax(token_stream, declaration_name, context.string_table)?;

    // Local declaration-ordering hints: declared type annotation only.
    // WHY: constant initializer references are now first-class ordering hints generated by
    // headers/constant_dependencies.rs; this function only collects type-surface hints.
    collect_constant_type_hints(
        &declaration_syntax,
        context,
        local_ordering_hints,
        capacity_references,
    );

    Ok(declaration_syntax)
}

fn internal_header_dispatch_error(
    message: &'static str,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerError::new(message, location, ErrorType::Compiler).into()
}
