//! Declaration-owned syntax for compiler build-configuration contracts.
//!
//! WHAT: parses the exact `#Config of T` qualifier into declaration metadata shared by header and
//! AST declaration parsing.
//! WHY: `#Config` is source syntax metadata, not a semantic type or expression category. Keeping
//! its parser here lets source contracts and anonymous const-record fields use one grammar owner.

use super::{DeclarationCursor, cursor_current_span};
use crate::compiler_frontend::build_config::{
    BuildInputName, BuildInputType, PrimitiveBuildInputType, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, DiagnosticToken, InvalidConfigReason,
    NumberLiteralErrorReason,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, parse_type_annotation_cursor,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::numeric_text::parse::{materialize_f64, materialize_i32};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenIndex, TokenRef, TokenTag,
};
/// Syntax metadata retained for a declaration carrying `#Config of T`.
///
/// The declaration's semantic type remains the parsed contract type. This value only preserves
/// the authored qualifier span until the owning semantic config pass consumes it.
#[derive(Clone, Debug)]
pub(crate) struct BuildConfigQualifierSyntax {
    pub(crate) type_annotation: ParsedTypeRef,
    pub(crate) qualifier_span: Option<SourceSpan>,
    pub(crate) default_none: bool,
}

impl BuildConfigQualifierSyntax {
    /// Remap all interned type names into the merged module string table.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.type_annotation.remap_string_ids(remap);
    }
}
/// One normalized source-owned `#Config` declaration shell.
///
/// The shell is collected while header syntax is prepared, before provider interfaces or AST
/// expression resolution exist. Its default is therefore either one already-materialized
/// primitive literal or the absence marker represented by `None`; no expression tree or provider
/// identity is retained here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceBuildConfigContract {
    pub(crate) name: BuildInputName,
    pub(crate) value_type: BuildInputType,
    pub(crate) required: bool,
    pub(crate) default: Option<PrimitiveBuildValue>,
    pub(crate) span: SourceSpan,
}

/// Convert one parsed type annotation into the build-input contract vocabulary.
///
/// Only one primitive or one optional primitive is accepted. Keeping this conversion beside the
/// qualifier grammar lets source-header preparation and direct project resolution share exactly
/// the same type boundary.
pub(crate) fn build_input_type_from_parsed(parsed: &ParsedTypeRef) -> Option<BuildInputType> {
    let primitive = match parsed {
        ParsedTypeRef::BuiltinString { .. } => PrimitiveBuildInputType::String,
        ParsedTypeRef::BuiltinInt { .. } => PrimitiveBuildInputType::Int,
        ParsedTypeRef::BuiltinFloat { .. } => PrimitiveBuildInputType::Float,
        ParsedTypeRef::BuiltinBool { .. } => PrimitiveBuildInputType::Bool,
        ParsedTypeRef::BuiltinChar { .. } => PrimitiveBuildInputType::Char,
        ParsedTypeRef::Optional { inner, .. } => {
            let primitive = match inner.as_ref() {
                ParsedTypeRef::BuiltinString { .. } => PrimitiveBuildInputType::String,
                ParsedTypeRef::BuiltinInt { .. } => PrimitiveBuildInputType::Int,
                ParsedTypeRef::BuiltinFloat { .. } => PrimitiveBuildInputType::Float,
                ParsedTypeRef::BuiltinBool { .. } => PrimitiveBuildInputType::Bool,
                ParsedTypeRef::BuiltinChar { .. } => PrimitiveBuildInputType::Char,
                _ => return None,
            };
            return Some(BuildInputType::Optional(primitive));
        }
        _ => return None,
    };
    Some(BuildInputType::Primitive(primitive))
}

/// Return the canonical source spelling for a build-input contract type.
pub(crate) fn build_input_type_name(value_type: BuildInputType) -> String {
    let primitive_name = value_type.primitive().name();
    if value_type.is_optional() {
        format!("{primitive_name}?")
    } else {
        primitive_name.to_owned()
    }
}

/// Return the authored span of a parsed type annotation for a structured config diagnostic.
pub(crate) fn parsed_type_span(parsed: &ParsedTypeRef) -> Option<SourceSpan> {
    match parsed {
        ParsedTypeRef::Named { span, .. }
        | ParsedTypeRef::Qualified { span, .. }
        | ParsedTypeRef::BuiltinBool { span, .. }
        | ParsedTypeRef::BuiltinInt { span, .. }
        | ParsedTypeRef::BuiltinFloat { span, .. }
        | ParsedTypeRef::BuiltinString { span, .. }
        | ParsedTypeRef::BuiltinChar { span, .. }
        | ParsedTypeRef::This { span, .. }
        | ParsedTypeRef::Optional { span, .. }
        | ParsedTypeRef::Collection { span, .. }
        | ParsedTypeRef::Map { span, .. }
        | ParsedTypeRef::Applied { span, .. } => *span,
        ParsedTypeRef::Inferred => None,
    }
}

/// Canonical source-token entry point for config literal normalization.
///
/// Each payload is read through its checked `TokenRef` accessor so malformed trusted records stay
/// in the infrastructure lane rather than being reported as user syntax.
pub(crate) fn normalize_source_build_config_contract_from_token(
    name: StringId,
    name_span: SourceSpan,
    qualifier: &BuildConfigQualifierSyntax,
    token: Option<TokenRef<'_>>,
    string_table: &mut StringTable,
) -> Result<SourceBuildConfigContract, HeaderParseFailure> {
    let name_text = string_table.resolve(name).to_owned();
    let input_name = BuildInputName::new(&name_text).map_err(|_| {
        HeaderParseFailure::Diagnostic(CompilerDiagnostic::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigContractNameInvalid,
            Some(name_span),
        ))
    })?;
    let value_type = build_input_type_from_parsed(&qualifier.type_annotation).ok_or_else(|| {
        HeaderParseFailure::Diagnostic(CompilerDiagnostic::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigQualifierUnsupportedType,
            parsed_type_span(&qualifier.type_annotation),
        ))
    })?;

    let (required, default) = match token {
        None => (!value_type.is_optional(), None),
        Some(token) => {
            let span = Some(token.source_span());
            match token.tag() {
                TokenTag::NONE_LITERAL => {
                    if !value_type.is_optional() {
                        return Err(HeaderParseFailure::Diagnostic(
                            source_default_type_mismatch(
                                name,
                                value_type,
                                "None",
                                span,
                                string_table,
                            ),
                        ));
                    }
                    (false, None)
                }
                TokenTag::STRING_SLICE_LITERAL => {
                    let spelling = token
                        .string_spelling(string_table)
                        .map_err(|error| {
                            HeaderParseFailure::Infrastructure(
                                CompilerDiagnostic::token_view_invariant_error(
                                    error,
                                    "source config string payload",
                                ),
                            )
                        })?
                        .ok_or_else(|| {
                            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                                "source config string token is missing its payload",
                            ))
                        })?;
                    validate_source_default_primitive(
                        name,
                        value_type,
                        PrimitiveBuildValue::String(spelling.to_owned()),
                        span,
                        string_table,
                    )
                    .map_err(HeaderParseFailure::Diagnostic)?
                }
                TokenTag::BOOL_LITERAL => {
                    let value = token.bool_value().ok_or_else(|| {
                        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                            "source config bool token has a malformed payload",
                        ))
                    })?;
                    validate_source_default_primitive(
                        name,
                        value_type,
                        PrimitiveBuildValue::Bool(value),
                        span,
                        string_table,
                    )
                    .map_err(HeaderParseFailure::Diagnostic)?
                }
                TokenTag::CHAR_LITERAL => {
                    let value = token.char_value().ok_or_else(|| {
                        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                            "source config char token has a malformed payload",
                        ))
                    })?;
                    validate_source_default_primitive(
                        name,
                        value_type,
                        PrimitiveBuildValue::Char(value),
                        span,
                        string_table,
                    )
                    .map_err(HeaderParseFailure::Diagnostic)?
                }
                TokenTag::NUMERIC_LITERAL => {
                    let numeric = token
                        .numeric_literal()
                        .map_err(|error| {
                            HeaderParseFailure::Infrastructure(
                                CompilerDiagnostic::token_view_invariant_error(
                                    error,
                                    "source config numeric payload",
                                ),
                            )
                        })?
                        .ok_or_else(|| {
                            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                                "source config numeric token is missing its payload",
                            ))
                        })?;
                    let materialized = match numeric.kind {
                        crate::compiler_frontend::numeric_text::token::NumericLiteralKind::WholeNumber => {
                            materialize_i32(numeric, string_table).map(PrimitiveBuildValue::Int)
                        }
                        crate::compiler_frontend::numeric_text::token::NumericLiteralKind::DecimalPoint
                        | crate::compiler_frontend::numeric_text::token::NumericLiteralKind::Exponent => {
                            materialize_f64(numeric, string_table).and_then(|number| {
                                PrimitiveBuildValue::float(number)
                                    .map_err(|_| NumberLiteralErrorReason::NonFiniteFloat)
                            })
                        }
                    };
                    let materialized = materialized.map_err(|reason| {
                        HeaderParseFailure::Diagnostic(CompilerDiagnostic::invalid_number_literal(
                            numeric.source_text,
                            reason,
                            span,
                        ))
                    })?;
                    validate_source_default_primitive(
                        name,
                        value_type,
                        materialized,
                        span,
                        string_table,
                    )
                    .map_err(HeaderParseFailure::Diagnostic)?
                }
                _ => {
                    return Err(HeaderParseFailure::Diagnostic(
                        source_default_type_mismatch(
                            name,
                            value_type,
                            "non-primitive",
                            span,
                            string_table,
                        ),
                    ));
                }
            }
        }
    };

    Ok(SourceBuildConfigContract {
        name: input_name,
        value_type,
        required,
        default,
        span: qualifier.qualifier_span.unwrap_or(name_span),
    })
}

/// Reject a source `#Config` initializer that contains more than one token.
///
/// The source contract stores only one already-materialized primitive. A longer initializer is
/// therefore a diagnosed non-primitive default rather than a truncated first-token value.
pub(crate) fn normalize_source_build_config_contract_non_primitive(
    name: StringId,
    name_span: SourceSpan,
    qualifier: &BuildConfigQualifierSyntax,
    span: SourceSpan,
    string_table: &mut StringTable,
) -> Result<SourceBuildConfigContract, HeaderParseFailure> {
    let name_text = string_table.resolve(name).to_owned();
    BuildInputName::new(&name_text).map_err(|_| {
        HeaderParseFailure::Diagnostic(CompilerDiagnostic::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigContractNameInvalid,
            Some(name_span),
        ))
    })?;
    let value_type = build_input_type_from_parsed(&qualifier.type_annotation).ok_or_else(|| {
        HeaderParseFailure::Diagnostic(CompilerDiagnostic::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigQualifierUnsupportedType,
            parsed_type_span(&qualifier.type_annotation),
        ))
    })?;
    Err(HeaderParseFailure::Diagnostic(
        source_default_type_mismatch(name, value_type, "non-primitive", Some(span), string_table),
    ))
}

fn validate_source_default_primitive(
    name: StringId,
    value_type: BuildInputType,
    value: PrimitiveBuildValue,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> Result<(bool, Option<PrimitiveBuildValue>), CompilerDiagnostic> {
    if !value_type.accepts_primitive(value.primitive_type()) {
        return Err(source_default_type_mismatch(
            name,
            value_type,
            value.primitive_type().name(),
            span,
            string_table,
        ));
    }
    Ok((false, Some(value)))
}

fn source_default_type_mismatch(
    name: StringId,
    value_type: BuildInputType,
    provided: &str,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_config_reason(
        Some(name),
        InvalidConfigReason::ConfigInputTypeMismatch {
            provided: string_table.intern(provided),
            expected: string_table.intern(&build_input_type_name(value_type)),
            provided_argument_index: None,
        },
        span,
    )
}

/// Bounded `#Config` marker scan over one source-owned cursor view.
///
/// WHAT: inspects adjacent pairs from a checked cursor without materializing the
///       range or retaining adapter vectors.
/// WHY: the outer file parser's start-body ranges and retained header ranges are
///      source-owned; per-range vector projection would pay O(N) allocation for a
///      two-token marker fact.
pub(crate) fn find_config_qualifier_marker_in_cursor(
    mut cursor: TokenCursor<'_>,
    string_table: &StringTable,
    span_builder: &ExtendedSpanBuilder,
) -> Option<(SourceSpan, bool)> {
    let resolver = span_builder.resolver();
    let mut previous = cursor.advance()?;
    loop {
        let crosses_segment = cursor.is_at_segment_start();
        let current = cursor.advance()?;
        let is_eof = current.is_eof();
        if !crosses_segment
            && previous.tag() == TokenTag::HASH
            && current.tag() == TokenTag::SYMBOL
            && current
                .string_id()
                .is_some_and(|name| string_table.resolve(name) == "Config")
        {
            let marker = previous.source_span();
            let marker_range = previous.span().resolve_with(resolver);
            let config_range = current.span().resolve_with(resolver);
            return Some((marker, marker_range.end() == config_range.start()));
        }
        if is_eof {
            return None;
        }
        previous = current;
    }
}

/// Bounded invalid-spacing scan over one checked source-owned cursor.
///
/// The direct project-config path uses this helper before header preparation. It observes exact
/// canonical spans without materializing a token vector; the normal declaration-shell path
/// validates adjacency at its parser cursor.
pub(crate) fn find_invalid_config_qualifier_spacing_in_cursor(
    mut cursor: TokenCursor<'_>,
    string_table: &StringTable,
    span_builder: &ExtendedSpanBuilder,
) -> Option<SourceSpan> {
    let resolver = span_builder.resolver();
    let mut previous = cursor.advance()?;
    loop {
        let crosses_segment = cursor.is_at_segment_start();
        let current = cursor.advance()?;
        let is_eof = current.is_eof();
        if !crosses_segment
            && previous.tag() == TokenTag::HASH
            && current.tag() == TokenTag::SYMBOL
            && current
                .string_id()
                .is_some_and(|name| string_table.resolve(name) == "Config")
        {
            let marker_range = previous.span().resolve_with(resolver);
            let config_range = current.span().resolve_with(resolver);
            if marker_range.end() != config_range.start() {
                return Some(previous.source_span());
            }
        }
        if is_eof {
            return None;
        }
        previous = current;
    }
}

/// Canonical source-view entry for header-owned `#Config` dispatch.
///
/// The declaration parser reads canonical source tags and source-owned symbol payloads directly;
/// qualifier scanning stays bounded to the active source view without a compatibility cursor.
pub(crate) fn starts_build_config_qualifier_at_source(
    source_tokens: &SourceTokens,
    index: TokenIndex,
    string_table: &StringTable,
) -> bool {
    let Some(current) = source_tokens.token(index).ok() else {
        return false;
    };
    if current.tag() != TokenTag::HASH {
        return false;
    }
    let Some(next_raw) = index.raw().checked_add(1) else {
        return false;
    };
    let Some(next_index) = TokenIndex::try_from_raw(next_raw) else {
        return false;
    };
    source_tokens.token(next_index).ok().is_some_and(|token| {
        token.tag() == TokenTag::SYMBOL
            && token
                .string_id()
                .is_some_and(|name| string_table.resolve(name) == "Config")
    })
}

/// Return whether a canonical cursor begins the compiler-owned `#Config` spelling.
pub(crate) fn starts_build_config_qualifier_at_cursor(
    cursor: &DeclarationCursor<'_>,
    string_table: &mut StringTable,
) -> Result<bool, crate::compiler_frontend::compiler_errors::CompilerError> {
    if cursor.current_tag() != TokenTag::HASH {
        return Ok(false);
    }
    Ok(cursor
        .peek_next_string_id_in(string_table)?
        .is_some_and(|name| string_table.resolve(name) == "Config"))
}

/// Parse the exact structural `#Config of T` qualifier.
pub(crate) fn parse_build_config_qualifier(
    token_stream: &mut DeclarationCursor<'_>,
    string_table: &mut StringTable,
    span_builder: Option<&mut ExtendedSpanBuilder>,
) -> Result<BuildConfigQualifierSyntax, HeaderParseFailure> {
    let qualifier_span = cursor_current_span(&token_stream.canonical_cursor());
    if let Some(span_builder) = span_builder {
        require_config_marker_adjacent(token_stream, span_builder)?;
    }
    if token_stream.current_tag() == TokenTag::HASH {
        token_stream.advance();
    }

    let is_config_symbol = if token_stream.current_tag() == TokenTag::SYMBOL {
        token_stream
            .current_string_id_in(string_table)
            .map_err(HeaderParseFailure::Infrastructure)?
            .is_some_and(|name| string_table.resolve(name) == "Config")
    } else {
        false
    };
    if is_config_symbol {
        token_stream.advance();
    } else {
        let expected =
            DiagnosticToken::from_string_tag(TokenTag::SYMBOL, string_table.intern("Config"));
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                HeaderParseFailure::Infrastructure(CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "config qualifier diagnostic projection",
                ))
            })?
            .or_else(|| Some(DiagnosticToken::from_static_tag(TokenTag::EOF)));
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::expected_token_from_projections(
                expected,
                found,
                token_stream.current_span(),
            ),
        ));
    }
    if token_stream.current_tag() != TokenTag::OF {
        let span = token_stream.current_span();
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                HeaderParseFailure::Infrastructure(CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "config qualifier diagnostic projection",
                ))
            })?
            .or_else(|| Some(DiagnosticToken::from_static_tag(TokenTag::EOF)));
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::expected_token_from_tags(TokenTag::OF, found, span),
        ));
    }
    token_stream.advance();

    let type_annotation = parse_type_annotation_cursor(
        token_stream,
        TypeAnnotationContext::BuildConfigContract,
        string_table,
    )?;

    Ok(BuildConfigQualifierSyntax {
        type_annotation,
        qualifier_span,
        default_none: false,
    })
}

/// `#Config` has a qualifier-specific spacing rule, distinct from ordinary `#` bindings.
fn require_config_marker_adjacent(
    token_stream: &DeclarationCursor<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<(), HeaderParseFailure> {
    let Some(current_token) = token_stream.canonical_cursor().current() else {
        return Ok(());
    };
    let Some(next_token) = token_stream.canonical_cursor().peek_next() else {
        return Ok(());
    };

    let resolver = span_builder.resolver();
    let current_range = current_token.span().resolve_with(resolver);
    let next_range = next_token.span().resolve_with(resolver);
    if current_range.end() != next_range.start() {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::common_syntax_mistake(
                CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing,
                Some(current_token.source_span()),
            ),
        ));
    }

    Ok(())
}
