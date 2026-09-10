//! Declaration-owned syntax for compiler build-configuration contracts.
//!
//! WHAT: parses the exact `#Config of T` qualifier into declaration metadata shared by header and
//! AST declaration parsing.
//! WHY: `#Config` is source syntax metadata, not a semantic type or expression category. Keeping
//! its parser here lets source contracts and anonymous const-record fields use one grammar owner.

use crate::compiler_frontend::build_config::{
    BuildInputName, BuildInputType, PrimitiveBuildInputType, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, InvalidConfigReason, NumberLiteralErrorReason,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, parse_type_annotation,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::numeric_text::parse::{materialize_f64, materialize_i32};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
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

/// Normalize one top-level source declaration's qualifier and literal initializer.
///
/// This deliberately consumes only retained tokens. In particular it does not construct an AST
/// expression, consult a scope, or invoke template/constant evaluation. A declaration without an
/// initializer is a required shell; `none` is accepted only for an optional contract.
pub(crate) fn normalize_source_build_config_contract(
    name: StringId,
    name_span: SourceSpan,
    qualifier: &BuildConfigQualifierSyntax,
    initializer_tokens: &[Token],
    string_table: &mut StringTable,
) -> Result<SourceBuildConfigContract, CompilerDiagnostic> {
    let name_text = string_table.resolve(name).to_owned();
    let input_name = BuildInputName::new(&name_text).map_err(|_| {
        CompilerDiagnostic::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigContractNameInvalid,
            Some(name_span),
        )
    })?;
    let value_type = build_input_type_from_parsed(&qualifier.type_annotation).ok_or_else(|| {
        CompilerDiagnostic::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigQualifierUnsupportedType,
            parsed_type_span(&qualifier.type_annotation),
        )
    })?;

    let source_id = name_span.source();
    let token_span = |token: &Token| Some(SourceSpan::new(source_id, token.span));
    let (required, default) = if initializer_tokens.is_empty() {
        (!value_type.is_optional(), None)
    } else if initializer_tokens.len() != 1 {
        return Err(source_default_type_mismatch(
            name,
            value_type,
            "non-primitive",
            initializer_tokens
                .first()
                .and_then(token_span)
                .or(qualifier.qualifier_span)
                .or(Some(name_span)),
            string_table,
        ));
    } else {
        let token = &initializer_tokens[0];
        let span = token_span(token);
        match &token.kind {
            TokenKind::NoneLiteral => {
                if !value_type.is_optional() {
                    return Err(source_default_type_mismatch(
                        name,
                        value_type,
                        "None",
                        span,
                        string_table,
                    ));
                }
                (false, None)
            }
            TokenKind::StringSliceLiteral(value) => {
                let value = PrimitiveBuildValue::String(string_table.resolve(*value).to_owned());
                validate_source_default_primitive(name, value_type, value, span, string_table)?
            }
            TokenKind::BoolLiteral(value) => validate_source_default_primitive(
                name,
                value_type,
                PrimitiveBuildValue::Bool(*value),
                span,
                string_table,
            )?,
            TokenKind::CharLiteral(value) => validate_source_default_primitive(
                name,
                value_type,
                PrimitiveBuildValue::Char(*value),
                span,
                string_table,
            )?,
            TokenKind::NumericLiteral(value) => {
                let materialized = match value.kind {
                    crate::compiler_frontend::numeric_text::token::NumericLiteralKind::WholeNumber => {
                        materialize_i32(value, string_table).map(PrimitiveBuildValue::Int)
                    }
                    crate::compiler_frontend::numeric_text::token::NumericLiteralKind::DecimalPoint
                    | crate::compiler_frontend::numeric_text::token::NumericLiteralKind::Exponent => {
                        materialize_f64(value, string_table).and_then(|number| {
                            PrimitiveBuildValue::float(number).map_err(|_| {
                                NumberLiteralErrorReason::NonFiniteFloat
                            })
                        })
                    }
                };
                let materialized = materialized.map_err(|reason| {
                    CompilerDiagnostic::invalid_number_literal(value.source_text, reason, span)
                })?;
                validate_source_default_primitive(
                    name,
                    value_type,
                    materialized,
                    span,
                    string_table,
                )?
            }
            _ => {
                return Err(source_default_type_mismatch(
                    name,
                    value_type,
                    "non-primitive",
                    span,
                    string_table,
                ));
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

/// Find one `#Config` marker in a retained token slice.
///
/// Header preparation uses this flat scan to reject body and nested placements before AST
/// construction. It reports the marker's exact source-qualified span and whether the following
/// `Config` token is byte-adjacent.
pub(crate) fn find_config_qualifier_marker(
    tokens: &[Token],
    string_table: &StringTable,
    source_id: SourceId,
    span_builder: &ExtendedSpanBuilder,
) -> Option<(SourceSpan, bool)> {
    let resolver = span_builder.resolver();
    for pair in tokens.windows(2) {
        if pair[0].kind != TokenKind::Hash {
            continue;
        }
        let TokenKind::Symbol(name) = pair[1].kind else {
            continue;
        };
        if string_table.resolve(name) != "Config" {
            continue;
        }

        let marker = SourceSpan::new(source_id, pair[0].span);
        let marker_range = pair[0].span.resolve_with(resolver);
        let config_range = pair[1].span.resolve_with(resolver);
        return Some((marker, marker_range.end() == config_range.start()));
    }

    None
}
/// Find a `#Config` marker whose tokens are separated by trivia.
///
/// The direct project-config AST path parses anonymous record fields without the retained
/// preparation span builder. It therefore preflights the token stream here, while the normal
/// declaration-shell path continues to validate adjacency at its parser cursor.
pub(crate) fn find_invalid_config_qualifier_spacing(
    tokens: &[Token],
    string_table: &StringTable,
    source_id: SourceId,
    span_builder: &ExtendedSpanBuilder,
) -> Option<SourceSpan> {
    let resolver = span_builder.resolver();
    tokens.windows(2).find_map(|pair| {
        if pair[0].kind != TokenKind::Hash {
            return None;
        }
        let TokenKind::Symbol(name) = pair[1].kind else {
            return None;
        };
        if string_table.resolve(name) != "Config" {
            return None;
        }

        let marker_range = pair[0].span.resolve_with(resolver);
        let config_range = pair[1].span.resolve_with(resolver);
        (marker_range.end() != config_range.start())
            .then(|| SourceSpan::new(source_id, pair[0].span))
    })
}

/// Returns whether the cursor begins the compiler-owned `#Config` qualifier spelling.
///
/// The lookahead intentionally ignores adjacency. `# Config` must enter the qualifier parser so
/// it can produce the dedicated qualifier-spacing diagnostic instead of ordinary `#` binding
/// diagnostics.
pub(crate) fn starts_build_config_qualifier(
    token_stream: &FileTokens,
    string_table: &StringTable,
) -> bool {
    if token_stream.current_token_kind() != &TokenKind::Hash {
        return false;
    }

    matches!(
        token_stream.peek_next_token(),
        Some(TokenKind::Symbol(name)) if string_table.resolve(*name) == "Config"
    )
}

/// Parse the exact structural `#Config of T` qualifier.
///
/// Contract types use their own required type-annotation context. In particular, an assignment or
pub(crate) fn parse_build_config_qualifier(
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
    span_builder: Option<&mut ExtendedSpanBuilder>,
) -> Result<BuildConfigQualifierSyntax, HeaderParseFailure> {
    let qualifier_span = Some(SourceSpan::new(
        token_stream.file_id,
        token_stream.current_token().span,
    ));
    if let Some(span_builder) = span_builder {
        require_config_marker_adjacent(token_stream, span_builder)?;
    }
    if token_stream.current_token_kind() == &TokenKind::Hash {
        token_stream.advance();
    }

    match token_stream.current_token_kind() {
        TokenKind::Symbol(name) if string_table.resolve(*name) == "Config" => {
            token_stream.advance();
        }
        _ => {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::expected_token(
                    TokenKind::Symbol(string_table.intern("Config")),
                    Some(token_stream.current_token_kind().to_owned()),
                    Some(SourceSpan::new(
                        token_stream.file_id,
                        token_stream.current_token().span,
                    )),
                ),
            ));
        }
    }

    if token_stream.current_token_kind() != &TokenKind::Of {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::expected_token(
                TokenKind::Of,
                Some(token_stream.current_token_kind().to_owned()),
                Some(SourceSpan::new(
                    token_stream.file_id,
                    token_stream.current_token().span,
                )),
            ),
        ));
    }
    token_stream.advance();

    let type_annotation = parse_type_annotation(
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
    token_stream: &FileTokens,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<(), HeaderParseFailure> {
    let Some(current_token) = token_stream.tokens.get(token_stream.index) else {
        return Ok(());
    };
    let Some(next_token) = token_stream.tokens.get(token_stream.index + 1) else {
        return Ok(());
    };

    let resolver = span_builder.resolver();
    let current_range = current_token.span.resolve_with(resolver);
    let next_range = next_token.span.resolve_with(resolver);
    if current_range.end() != next_range.start() {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::common_syntax_mistake(
                CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing,
                Some(SourceSpan::new(token_stream.file_id, current_token.span)),
            ),
        ));
    }

    Ok(())
}
