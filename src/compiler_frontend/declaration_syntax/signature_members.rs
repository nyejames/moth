//! Neutral `| ... |` signature and record member shell parsing.
//!
//! WHAT: parses function parameters, function returns, struct fields, and choice payload fields
//! into syntax shells that preserve parsed type references and default-expression tokens.
//! WHY: header parsing owns declaration-shell discovery, but AST owns type resolution and
//! expression parsing. Keeping this module AST-free preserves that stage boundary.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFunctionSignatureReason, InvalidSignatureMemberReason,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::declaration_shell::require_binding_marker_adjacent;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, parse_type_annotation,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::syntax_errors::signature_position::check_signature_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
use crate::compiler_frontend::utilities::token_scan::NestingDepth;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

/// Two-lane result for signature member/parameter parsing.
///
/// WHAT: keeps parameter and member shell parsing on one small error boundary while
///       preserving structured diagnostics for header and AST callers, with infrastructure
///       failures aborting through the typed lane.
/// WHY: these connected parsers otherwise carry the large diagnostic value
///      through every successful header parse. Plain callers extract the
///      inline diagnostic lane once.
type SignatureMemberParseResult<T> = Result<T, HeaderParseFailure>;

/// Distinguishes the two syntactic contexts that share `| ... |` member parsing.
///
/// WHAT: `this` is valid only in function parameter lists, not in struct fields.
/// WHY: the shell parser is shared, but legal names and defaults differ by context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureMemberContext {
    FunctionParameter,
    StructField,
    ChoicePayloadField,
    TraitRequirement,
}

/// One parsed parameter/field shell before AST type resolution.
#[derive(Clone, Debug)]
pub struct SignatureMemberSyntax {
    pub id: PathId,
    pub value_mode: ValueMode,
    pub is_reactive: bool,
    pub type_annotation: ParsedTypeRef,
    pub default_tokens: Vec<Token>,
    pub span: Option<SourceSpan>,
}

/// Function return-channel syntax before it becomes an AST `ReturnChannel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnChannelSyntax {
    Success,
    Error,
}

/// One parsed function return item before AST type resolution.
#[derive(Clone, Debug)]
pub struct FunctionReturnSyntax {
    pub type_annotation: ParsedTypeRef,
    pub span: Option<SourceSpan>,
}

#[derive(Clone, Debug)]
pub struct ReturnSlotSyntax {
    pub value: FunctionReturnSyntax,
    pub channel: ReturnChannelSyntax,
}

/// Parsed function signature shell.
#[derive(Clone, Debug, Default)]
pub struct FunctionSignatureSyntax {
    pub parameters: Vec<SignatureMemberSyntax>,
    pub returns: Vec<ReturnSlotSyntax>,
}

impl SignatureMemberSyntax {
    /// Remap all interned names, type refs, and tokens.
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.type_annotation.remap_string_ids(remap);
        for token in &mut self.default_tokens {
            token.remap_string_ids(remap);
        }
    }

    /// Remap the member path identity after its path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.id = remap.get(self.id);
    }

    pub fn validate_required_source_prefix(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        if !path_fork.starts_with(self.id, provisional_source_file) {
            return Err(CompilerError::compiler_error(
                "source-owned retained path is missing its provisional source prefix",
            ));
        }
        Ok(())
    }
}

impl FunctionReturnSyntax {
    /// Remap return type references and source locations into the merged string table.
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.type_annotation.remap_string_ids(remap);
    }
}

impl FunctionSignatureSyntax {
    /// Remap all interned string IDs in the signature shell.
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for parameter in &mut self.parameters {
            parameter.remap_string_ids(remap);
        }
        for return_slot in &mut self.returns {
            return_slot.value.remap_string_ids(remap);
        }
    }

    /// Remap member path identities after the path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        for parameter in &mut self.parameters {
            parameter.remap_path_ids(remap);
        }
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        for parameter in &self.parameters {
            parameter.validate_required_source_prefix(provisional_source_file, path_fork)?;
        }
        Ok(())
    }
}

/// Parse a full function signature shell from `| params | -> returns:`.
///
/// ENTRY INVARIANT: the stream is positioned on the opening `|`.
/// EXIT INVARIANT: the stream is positioned immediately after the terminating `:`.
pub fn parse_function_signature_syntax(
    token_stream: &mut FileTokens,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
    function_path: PathId,
    path_fork: &mut PathInternerFork,
    span_builder: &mut ExtendedSpanBuilder,
) -> SignatureMemberParseResult<FunctionSignatureSyntax> {
    token_stream.advance();

    let parameters = parse_signature_members_syntax(
        token_stream,
        string_table,
        warnings,
        SignatureMemberContext::FunctionParameter,
        function_path,
        path_fork,
        span_builder,
    )?;
    token_stream.advance();

    match token_stream.current_token_kind() {
        TokenKind::Arrow => {}

        TokenKind::Colon => {
            token_stream.advance();
            return Ok(FunctionSignatureSyntax {
                parameters,
                returns: Vec::new(),
            });
        }

        TokenKind::DatatypeInt
        | TokenKind::DatatypeFloat
        | TokenKind::DatatypeBool
        | TokenKind::DatatypeString
        | TokenKind::DatatypeChar
        | TokenKind::DatatypeNone
        | TokenKind::OpenCurly
        | TokenKind::Symbol(_) => {
            return Err(CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingArrowOrColon {
                    found: token_stream.current_token_kind().clone().into(),
                },
                current_source_span(token_stream),
            )
            .into());
        }

        TokenKind::Newline | TokenKind::Eof | TokenKind::End => {
            return Err(CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::UnexpectedEndAfterParameters,
                current_source_span(token_stream),
            )
            .into());
        }

        _ => {
            return Err(CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingArrowOrColon {
                    found: token_stream.current_token_kind().clone().into(),
                },
                current_source_span(token_stream),
            )
            .into());
        }
    }

    let returns = parse_return_list_syntax(token_stream, &parameters, string_table)?;

    Ok(FunctionSignatureSyntax {
        parameters,
        returns,
    })
}

/// Parses a `| name [~]Type [= default], ... |` member list into neutral shells.
///
pub fn parse_signature_members_syntax(
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
    warnings: &mut Vec<CompilerDiagnostic>,
    member_context: SignatureMemberContext,
    owner_path: PathId,
    path_fork: &mut PathInternerFork,
    span_builder: &mut ExtendedSpanBuilder,
) -> SignatureMemberParseResult<Vec<SignatureMemberSyntax>> {
    let mut members = Vec::with_capacity(1);
    let mut expecting_member = true;
    let mut member_index = 0;
    let mut seen_member_names: FxHashMap<StringId, SourceSpan> = FxHashMap::default();

    fn ensure_member_slot(
        expecting_member: bool,
        token_stream: &FileTokens,
    ) -> SignatureMemberParseResult<()> {
        if !expecting_member {
            return Err(CompilerDiagnostic::expected_token(
                TokenKind::Comma,
                Some(token_stream.current_token_kind().to_owned()),
                current_source_span(token_stream),
            )
            .into());
        }

        Ok(())
    }

    while token_stream.index < token_stream.tokens.len() {
        match token_stream.current_token_kind().to_owned() {
            TokenKind::TypeParameterBracket => {
                return Ok(members);
            }

            TokenKind::End => {
                return Err(CompilerDiagnostic::unexpected_end_of_file(
                    None,
                    current_source_span(token_stream),
                )
                .into());
            }

            TokenKind::Arrow | TokenKind::Colon => {
                return Err(CompilerDiagnostic::unexpected_token(
                    token_stream.current_token_kind().to_owned(),
                    current_source_span(token_stream),
                )
                .into());
            }

            TokenKind::Symbol(member_name) => {
                ensure_member_slot(expecting_member, token_stream)?;

                let member_path = path_fork
                    .try_intern_child(owner_path, member_name)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "path table exhausted while interning a signature member path",
                        )
                    })?;
                let member = parse_signature_member_syntax(
                    token_stream,
                    member_path,
                    string_table,
                    warnings,
                    false,
                    member_context,
                    path_fork,
                    span_builder,
                )?;

                record_ordinary_member_name(&mut seen_member_names, &member, path_fork)?;
                members.push(member);
                expecting_member = false;
                member_index += 1;
            }

            TokenKind::This if member_context == SignatureMemberContext::FunctionParameter => {
                ensure_member_slot(expecting_member, token_stream)?;

                let this_id = string_table.intern("this");
                let this_path = path_fork
                    .try_intern_child(owner_path, this_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "path table exhausted while interning a signature member path",
                        )
                    })?;
                let member = parse_signature_member_syntax(
                    token_stream,
                    this_path,
                    string_table,
                    warnings,
                    true,
                    member_context,
                    path_fork,
                    span_builder,
                )?;

                members.push(member);
                expecting_member = false;
                member_index += 1;
            }

            TokenKind::This => {
                return Err(CompilerDiagnostic::invalid_signature_member(
                    InvalidSignatureMemberReason::ThisNotAllowed,
                    current_source_span(token_stream),
                )
                .into());
            }

            TokenKind::TraitThis if member_context == SignatureMemberContext::TraitRequirement => {
                ensure_member_slot(expecting_member, token_stream)?;

                if member_index > 0 {
                    return Err(CompilerDiagnostic::invalid_signature_member(
                        InvalidSignatureMemberReason::TraitBareThisOnlyReceiver,
                        current_source_span(token_stream),
                    )
                    .into());
                }

                let this_id = string_table.intern("This");
                let this_path = path_fork
                    .try_intern_child(owner_path, this_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "path table exhausted while interning a signature member path",
                        )
                    })?;
                let member = parse_trait_this_member_syntax(
                    token_stream,
                    this_path,
                    ValueMode::ImmutableOwned,
                    span_builder,
                )?;

                members.push(member);
                expecting_member = false;
                member_index += 1;
            }

            TokenKind::Mutable if member_context == SignatureMemberContext::TraitRequirement => {
                ensure_member_slot(expecting_member, token_stream)?;

                token_stream.advance();

                if token_stream.current_token_kind() != &TokenKind::TraitThis {
                    return Err(CompilerDiagnostic::invalid_signature_member(
                        InvalidSignatureMemberReason::TraitReceiverMustBeThis,
                        current_source_span(token_stream),
                    )
                    .into());
                }

                if member_index > 0 {
                    return Err(CompilerDiagnostic::invalid_signature_member(
                        InvalidSignatureMemberReason::TraitMutableThisOnlyFirstParameter,
                        current_source_span(token_stream),
                    )
                    .into());
                }

                let this_id = string_table.intern("This");
                let this_path = path_fork
                    .try_intern_child(owner_path, this_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "path table exhausted while interning a signature member path",
                        )
                    })?;
                let member = parse_trait_this_member_syntax(
                    token_stream,
                    this_path,
                    ValueMode::MutableOwned,
                    span_builder,
                )?;

                members.push(member);
                expecting_member = false;
                member_index += 1;
            }

            TokenKind::Comma => {
                token_stream.advance();
                if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
                    return Err(CompilerDiagnostic::unexpected_trailing_comma(
                        current_source_span(token_stream),
                    )
                    .into());
                }
                expecting_member = true;
            }

            TokenKind::Must | TokenKind::TraitThis => {
                let keyword = reserved_trait_keyword_or_dispatch_mismatch(
                    token_stream.current_token_kind(),
                    current_source_span(token_stream),
                    "Struct/Parameter Parsing",
                    "signature member parsing",
                )?;

                return Err(reserved_trait_keyword_error(
                    keyword,
                    current_source_span(token_stream),
                )
                .into());
            }

            TokenKind::Newline => {
                token_stream.advance();
            }

            TokenKind::Eof => {
                return Err(CompilerDiagnostic::unexpected_end_of_file(
                    None,
                    current_source_span(token_stream),
                )
                .into());
            }

            _ => {
                if let Some(error) = check_signature_common_mistake(token_stream) {
                    return Err(error.into());
                }

                return Err(CompilerDiagnostic::unexpected_token(
                    token_stream.current_token_kind().to_owned(),
                    current_source_span(token_stream),
                )
                .into());
            }
        }
    }

    Ok(members)
}

/// Reject a duplicate member name in one shared `| ... |` member list.
///
/// WHAT: the current member is primary and the first member is secondary, reusing
/// `DuplicateDeclaration` (`MOTH-RULE-0002`) so ordinary function parameters, struct fields,
/// choice payload fields and ordinary trait-requirement parameters share one owner. Reserved
/// receivers keep their receiver-specific validation.
/// WHY: keeping duplicate-name detection in the shared signature-member parser prevents
/// duplicate members from reaching HIR or infrastructure invariants, and avoids
/// function-, struct- or choice-specific duplicate validators.
fn record_ordinary_member_name(
    seen_member_names: &mut FxHashMap<StringId, SourceSpan>,
    member: &SignatureMemberSyntax,
    path_fork: &PathInternerFork,
) -> SignatureMemberParseResult<()> {
    let Some(member_name) = path_fork.component(member.id) else {
        return Ok(());
    };

    if let Some(first_span) = seen_member_names.get(&member_name) {
        return Err(CompilerDiagnostic::duplicate_declaration(
            member_name,
            Some(*first_span),
            member.span,
        )
        .into());
    }

    if let Some(member_span) = member.span {
        seen_member_names.insert(member_name, member_span);
    }
    Ok(())
}
fn parse_signature_member_syntax(
    token_stream: &mut FileTokens,
    full_name: PathId,
    string_table: &mut StringTable,
    warnings: &mut Vec<CompilerDiagnostic>,
    allow_reserved_this: bool,
    member_context: SignatureMemberContext,
    path_fork: &PathInternerFork,
    span_builder: &mut ExtendedSpanBuilder,
) -> SignatureMemberParseResult<SignatureMemberSyntax> {
    let member_span = current_source_span(token_stream);
    if !allow_reserved_this && let Some(name_id) = path_fork.component(full_name) {
        ensure_not_keyword_shadow_identifier(name_id, member_span, string_table)?;
    }

    if let Some(name_id) = path_fork.component(full_name)
        && let Some(warning) = naming_warning_for_identifier(
            name_id,
            member_span,
            IdentifierNamingKind::ValueLike,
            string_table,
        )
    {
        warnings.push(warning);
    }

    token_stream.advance();

    let mut value_mode = ValueMode::ImmutableOwned;
    let mut is_reactive = false;
    match token_stream.current_token_kind() {
        TokenKind::Mutable => {
            token_stream.advance();
            value_mode = ValueMode::MutableOwned;
        }

        TokenKind::Reactive => {
            if member_context != SignatureMemberContext::FunctionParameter {
                return Err(CompilerDiagnostic::invalid_signature_member(
                    InvalidSignatureMemberReason::ReactiveAccessNotAllowed,
                    current_source_span(token_stream),
                )
                .into());
            }

            require_binding_marker_adjacent(
                token_stream,
                BindingMode::ReactiveRuntime,
                span_builder,
            )?;
            token_stream.advance();
            is_reactive = true;
        }

        _ => {}
    }

    if token_stream.current_token_kind() == &TokenKind::Hash {
        return Err(CompilerDiagnostic::invalid_signature_member(
            InvalidSignatureMemberReason::CompileTimeParameterDeferred,
            current_source_span(token_stream),
        )
        .into());
    }

    if member_context == SignatureMemberContext::ChoicePayloadField
        && value_mode == ValueMode::MutableOwned
    {
        return Err(CompilerDiagnostic::invalid_signature_member(
            InvalidSignatureMemberReason::ChoicePayloadMutable,
            current_source_span(token_stream),
        )
        .into());
    }

    while token_stream.current_token_kind() == &TokenKind::Newline {
        token_stream.advance();
    }

    let type_annotation = parse_type_annotation(
        token_stream,
        type_annotation_context_for_member(member_context),
        string_table,
    )?;
    let default_tokens = match token_stream.current_token_kind() {
        TokenKind::Assign => {
            token_stream.advance();
            if is_reactive {
                return Err(CompilerDiagnostic::invalid_signature_member(
                    InvalidSignatureMemberReason::ReactiveParameterDefaultValue,
                    current_source_span(token_stream),
                )
                .into());
            }
            if member_context == SignatureMemberContext::TraitRequirement {
                return Err(CompilerDiagnostic::invalid_signature_member(
                    InvalidSignatureMemberReason::TraitRequirementDefaultValue,
                    current_source_span(token_stream),
                )
                .into());
            }
            if member_context == SignatureMemberContext::ChoicePayloadField {
                return Err(CompilerDiagnostic::invalid_signature_member(
                    InvalidSignatureMemberReason::ChoicePayloadDefaultValue,
                    current_source_span(token_stream),
                )
                .into());
            }

            collect_member_default_tokens(token_stream)?
        }

        TokenKind::Comma
        | TokenKind::Eof
        | TokenKind::Newline
        | TokenKind::TypeParameterBracket => Vec::new(),

        TokenKind::As => {
            return Err(CompilerDiagnostic::unexpected_token(
                token_stream.current_token_kind().to_owned(),
                current_source_span(token_stream),
            )
            .into());
        }

        _ => {
            return Err(CompilerDiagnostic::unexpected_token(
                token_stream.current_token_kind().to_owned(),
                current_source_span(token_stream),
            )
            .into());
        }
    };

    Ok(SignatureMemberSyntax {
        id: full_name,
        value_mode,
        is_reactive,
        type_annotation,
        default_tokens,
        span: member_span,
    })
}

fn type_annotation_context_for_member(
    member_context: SignatureMemberContext,
) -> TypeAnnotationContext {
    match member_context {
        SignatureMemberContext::TraitRequirement => TypeAnnotationContext::TraitRequirement,
        SignatureMemberContext::FunctionParameter
        | SignatureMemberContext::StructField
        | SignatureMemberContext::ChoicePayloadField => TypeAnnotationContext::SignatureParameter,
    }
}

/// Parse a `This` receiver parameter in a trait requirement.
///
/// ENTRY INVARIANT: the stream is positioned on `This` (TraitThis).
/// EXIT INVARIANT: the stream is positioned on the token after `This`.
fn parse_trait_this_member_syntax(
    token_stream: &mut FileTokens,
    full_name: PathId,
    value_mode: ValueMode,
    _span_builder: &mut ExtendedSpanBuilder,
) -> SignatureMemberParseResult<SignatureMemberSyntax> {
    let member_span = current_source_span(token_stream);

    token_stream.advance(); // past This

    // Trait receiver parameters have no explicit type annotation;
    // the type is implicitly the implementing concrete type.
    let type_annotation = ParsedTypeRef::This { span: member_span };

    // Default values are not allowed in trait requirements.
    let default_tokens = Vec::new();

    Ok(SignatureMemberSyntax {
        id: full_name,
        value_mode,
        is_reactive: false,
        type_annotation,
        default_tokens,
        span: member_span,
    })
}

/// A token kind that can only follow an authored `=` when the default value is missing:
/// top-level comma, closing pipe, newline, block end or EOF.
fn is_missing_default_boundary(token_kind: &TokenKind) -> bool {
    matches!(
        token_kind,
        TokenKind::Comma
            | TokenKind::TypeParameterBracket
            | TokenKind::Newline
            | TokenKind::End
            | TokenKind::Eof
    )
}

fn collect_member_default_tokens(
    token_stream: &mut FileTokens,
) -> SignatureMemberParseResult<Vec<Token>> {
    // A member/EOF boundary before any expression token is a missing default, not an empty
    // one, so report it here rather than letting a newline reach the infra error path. Only
    // fires before the first expression token, so valid multiline defaults still pass.
    if is_missing_default_boundary(token_stream.current_token_kind()) {
        return Err(CompilerDiagnostic::invalid_signature_member(
            InvalidSignatureMemberReason::MissingDefaultValue,
            current_source_span(token_stream),
        )
        .into());
    }

    let mut tokens = Vec::new();
    let mut depth = NestingDepth::default();

    while token_stream.index < token_stream.length {
        let token_kind = token_stream.current_token_kind().clone();

        if depth.is_top_level()
            && matches!(
                token_kind,
                TokenKind::Comma | TokenKind::TypeParameterBracket | TokenKind::Eof
            )
        {
            break;
        }

        if matches!(token_kind, TokenKind::Eof) {
            return Err(CompilerDiagnostic::unexpected_end_of_file(
                None,
                current_source_span(token_stream),
            )
            .into());
        }

        depth.step(&token_kind);
        tokens.push(token_stream.current_token());
        token_stream.advance();
    }

    if tokens.is_empty() {
        return Err(CompilerDiagnostic::unexpected_token(
            token_stream.current_token_kind().to_owned(),
            current_source_span(token_stream),
        )
        .into());
    }

    Ok(tokens)
}

/// Parse a return list for a trait requirement, stopping at newline or block end.
///
/// ENTRY INVARIANT: the stream is positioned on the `->` arrow.
/// EXIT INVARIANT: the stream is positioned on the first token after the last return type.
fn parse_trait_requirement_return_list(
    token_stream: &mut FileTokens,
    parameters: &[SignatureMemberSyntax],
    string_table: &mut StringTable,
) -> SignatureMemberParseResult<Vec<ReturnSlotSyntax>> {
    let mut return_slots = Vec::new();

    token_stream.advance(); // past ->
    if let Some(diagnostic) = missing_return_type_after_arrow(
        token_stream,
        InvalidFunctionSignatureReason::MissingTraitRequirementReturnType,
    ) {
        return Err(diagnostic.into());
    }
    loop {
        return_slots.push(parse_single_return_item_syntax(
            token_stream,
            parameters,
            string_table,
            TypeAnnotationContext::TraitRequirement,
        )?);

        match token_stream.current_token_kind() {
            TokenKind::Comma => {
                let comma_span = current_source_span(token_stream);
                token_stream.advance();

                match token_stream.current_token_kind() {
                    TokenKind::Newline | TokenKind::End | TokenKind::Eof => {
                        return Err(
                            CompilerDiagnostic::unexpected_trailing_comma(comma_span).into()
                        );
                    }

                    _ => {}
                }
            }

            TokenKind::Newline | TokenKind::End | TokenKind::Eof => {
                return Ok(return_slots);
            }

            unexpected_token => {
                return Err(CompilerDiagnostic::invalid_function_signature(
                    InvalidFunctionSignatureReason::MissingCommaOrColon {
                        found: unexpected_token.clone().into(),
                    },
                    current_source_span(token_stream),
                )
                .into());
            }
        }
    }
}

/// Parse a trait requirement signature from `| params | [-> returns]`.
///
/// ENTRY INVARIANT: the stream is positioned on the opening `|`.
/// EXIT INVARIANT: the stream is positioned on the first token after the signature.
pub fn parse_trait_requirement_signature_syntax(
    token_stream: &mut FileTokens,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
    method_path: PathId,
    path_fork: &mut PathInternerFork,
    span_builder: &mut ExtendedSpanBuilder,
) -> SignatureMemberParseResult<FunctionSignatureSyntax> {
    token_stream.advance(); // past |
    let parameters = parse_signature_members_syntax(
        token_stream,
        string_table,
        warnings,
        SignatureMemberContext::TraitRequirement,
        method_path,
        path_fork,
        span_builder,
    )?;
    token_stream.advance(); // past |

    let returns = if token_stream.current_token_kind() == &TokenKind::Arrow {
        parse_trait_requirement_return_list(token_stream, &parameters, string_table)?
    } else {
        Vec::new()
    };

    Ok(FunctionSignatureSyntax {
        parameters,
        returns,
    })
}

/// Report a missing return type when the token after an authored `->` cannot start one.
///
/// WHAT: returns the caller-selected `reason` for `:`, newline, block end and EOF, the
///       boundaries where the signature parser owns the missing-type diagnosis instead of
///       delegating to a generic type-annotation error. The diagnostic points at the first
///       missing-type boundary after the arrow, never at the arrow itself.
/// WHY: function-signature and trait-requirement return lists share this boundary but need
///      different guidance. A function signature ends its body with `:`, while a bodyless
///      trait requirement does not. The caller selects the factual reason so the shared
///      boundary predicate stays in one place.
fn missing_return_type_after_arrow(
    token_stream: &FileTokens,
    reason: InvalidFunctionSignatureReason,
) -> Option<CompilerDiagnostic> {
    match token_stream.current_token_kind() {
        TokenKind::Colon | TokenKind::Newline | TokenKind::End | TokenKind::Eof => {
            Some(CompilerDiagnostic::invalid_function_signature(
                reason,
                current_source_span(token_stream),
            ))
        }
        _ => None,
    }
}

fn parse_return_list_syntax(
    token_stream: &mut FileTokens,
    parameters: &[SignatureMemberSyntax],
    string_table: &mut StringTable,
) -> SignatureMemberParseResult<Vec<ReturnSlotSyntax>> {
    let mut return_slots = Vec::new();

    token_stream.advance();
    if let Some(diagnostic) = missing_return_type_after_arrow(
        token_stream,
        InvalidFunctionSignatureReason::MissingReturnType,
    ) {
        return Err(diagnostic.into());
    }

    loop {
        return_slots.push(parse_single_return_item_syntax(
            token_stream,
            parameters,
            string_table,
            TypeAnnotationContext::SignatureReturn,
        )?);

        match token_stream.current_token_kind() {
            TokenKind::Comma => {
                let comma_span = current_source_span(token_stream);
                token_stream.advance();

                match token_stream.current_token_kind() {
                    TokenKind::Colon => {
                        return Err(CompilerDiagnostic::invalid_function_signature(
                            InvalidFunctionSignatureReason::TrailingCommaInReturns,
                            comma_span,
                        )
                        .into());
                    }

                    TokenKind::Newline | TokenKind::End | TokenKind::Eof => {
                        return Err(CompilerDiagnostic::invalid_function_signature(
                            InvalidFunctionSignatureReason::UnexpectedEndAfterComma,
                            comma_span,
                        )
                        .into());
                    }

                    _ => {}
                }
            }
            TokenKind::Symbol(symbol) if string_table.resolve(*symbol) == "where" => {
                return Err(CompilerDiagnostic::invalid_function_signature(
                    InvalidFunctionSignatureReason::GenericWhereConstraintsUnsupported,
                    current_source_span(token_stream),
                )
                .into());
            }
            TokenKind::Colon => {
                token_stream.advance();
                validate_return_slots_syntax(&return_slots, token_stream, string_table)?;
                return Ok(return_slots);
            }
            TokenKind::Eof => {
                return Err(CompilerDiagnostic::invalid_function_signature(
                    InvalidFunctionSignatureReason::UnexpectedEndInReturns,
                    current_source_span(token_stream),
                )
                .into());
            }
            TokenKind::Newline | TokenKind::End => {
                return Err(CompilerDiagnostic::invalid_function_signature(
                    InvalidFunctionSignatureReason::MissingColonAfterReturns,
                    current_source_span(token_stream),
                )
                .into());
            }
            TokenKind::Arrow => {
                return Err(CompilerDiagnostic::invalid_function_signature(
                    InvalidFunctionSignatureReason::UnexpectedArrowInReturns,
                    current_source_span(token_stream),
                )
                .into());
            }
            unexpected_token => {
                return Err(CompilerDiagnostic::invalid_function_signature(
                    InvalidFunctionSignatureReason::MissingCommaOrColon {
                        found: unexpected_token.clone().into(),
                    },
                    current_source_span(token_stream),
                )
                .into());
            }
        }
    }
}

fn parse_single_return_item_syntax(
    token_stream: &mut FileTokens,
    _parameters: &[SignatureMemberSyntax],
    string_table: &StringTable,
    type_context: TypeAnnotationContext,
) -> SignatureMemberParseResult<ReturnSlotSyntax> {
    parse_value_return_type_syntax(token_stream, string_table, type_context)
}

fn parse_value_return_type_syntax(
    token_stream: &mut FileTokens,
    string_table: &StringTable,
    type_context: TypeAnnotationContext,
) -> SignatureMemberParseResult<ReturnSlotSyntax> {
    let span = current_source_span(token_stream);
    let type_annotation = parse_type_annotation(token_stream, type_context, string_table)?;

    if parsed_type_ref_is_void(&type_annotation, string_table) {
        return Err(CompilerDiagnostic::invalid_function_signature(
            InvalidFunctionSignatureReason::VoidNotAllowed,
            span,
        )
        .into());
    }

    let channel = if token_stream.current_token_kind() == &TokenKind::Bang {
        token_stream.advance();
        ReturnChannelSyntax::Error
    } else {
        ReturnChannelSyntax::Success
    };

    Ok(ReturnSlotSyntax {
        value: FunctionReturnSyntax {
            type_annotation,
            span,
        },
        channel,
    })
}

fn validate_return_slots_syntax(
    returns: &[ReturnSlotSyntax],
    token_stream: &FileTokens,
    string_table: &StringTable,
) -> SignatureMemberParseResult<()> {
    let error_return_slots: Vec<(usize, &ReturnSlotSyntax)> = returns
        .iter()
        .enumerate()
        .filter(|(_, return_slot)| return_slot.channel == ReturnChannelSyntax::Error)
        .collect();

    if error_return_slots.len() > 1 {
        return Err(CompilerDiagnostic::invalid_function_signature(
            InvalidFunctionSignatureReason::MultipleErrorReturnSlots,
            current_source_span(token_stream),
        )
        .into());
    }

    if let Some((error_index, _)) = error_return_slots.first()
        && *error_index + 1 != returns.len()
    {
        return Err(CompilerDiagnostic::invalid_function_signature(
            InvalidFunctionSignatureReason::ErrorSlotNotLast,
            current_source_span(token_stream),
        )
        .into());
    }

    for return_slot in returns {
        if parsed_type_ref_is_void(&return_slot.value.type_annotation, string_table) {
            return Err(CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::VoidNotAllowed,
                current_source_span(token_stream),
            )
            .into());
        }
    }

    Ok(())
}

fn parsed_type_ref_is_void(type_ref: &ParsedTypeRef, string_table: &StringTable) -> bool {
    matches!(
        type_ref,
        ParsedTypeRef::Named { name, .. } if string_table.resolve(*name) == "Void"
    )
}

fn current_source_span(token_stream: &FileTokens) -> Option<SourceSpan> {
    token_stream
        .tokens
        .get(token_stream.index)
        .map(|token| SourceSpan::new(token_stream.file_id, token.span))
}

#[cfg(test)]
#[path = "tests/signature_member_duplicate_tests.rs"]
mod signature_member_duplicate_tests;
