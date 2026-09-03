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
use crate::compiler_frontend::numeric_text::parse::{materialize_f64, materialize_i32};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};
/// Syntax metadata retained for a declaration carrying `#Config of T`.
///
/// The declaration's semantic type remains the parsed contract type. This value only preserves
/// the authored qualifier identity and location until the owning semantic config pass consumes it.
#[derive(Clone, Debug)]
pub(crate) struct BuildConfigQualifierSyntax {
    pub(crate) type_annotation: ParsedTypeRef,
    pub(crate) qualifier_location: SourceLocation,
    pub(crate) default_none: bool,
}

impl BuildConfigQualifierSyntax {
    /// Remap all interned type names into the merged module string table.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.type_annotation.remap_string_ids(remap);
        self.qualifier_location.remap_string_ids(remap);
    }

    /// Rebind all authored locations to the final logical source identity.
    pub(crate) fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.type_annotation.rebind_source_identity(logical_path);
        self.qualifier_location.rebind_source_identity(logical_path);
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
    pub(crate) location: SourceLocation,
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

/// Return the authored span of a parsed type annotation for a structured config diagnostic.
pub(crate) fn parsed_type_location(parsed: &ParsedTypeRef) -> SourceLocation {
    match parsed {
        ParsedTypeRef::Named { location, .. }
        | ParsedTypeRef::Qualified { location, .. }
        | ParsedTypeRef::BuiltinBool { location }
        | ParsedTypeRef::BuiltinInt { location }
        | ParsedTypeRef::BuiltinFloat { location }
        | ParsedTypeRef::BuiltinString { location }
        | ParsedTypeRef::BuiltinChar { location }
        | ParsedTypeRef::BuiltinNone { location }
        | ParsedTypeRef::This { location }
        | ParsedTypeRef::Optional { location, .. }
        | ParsedTypeRef::Collection { location, .. }
        | ParsedTypeRef::Map { location, .. }
        | ParsedTypeRef::Result { location, .. }
        | ParsedTypeRef::Applied { location, .. } => location.clone(),
        ParsedTypeRef::Inferred => SourceLocation::default(),
    }
}

/// Render the normalized contract type in the spelling used by diagnostics.
pub(crate) fn build_input_type_name(contract: BuildInputType) -> String {
    let primitive = contract.primitive().name();
    if contract.is_optional() {
        format!("{primitive}?")
    } else {
        primitive.to_owned()
    }
}

/// Normalize one top-level source declaration's qualifier and literal initializer.
///
/// This deliberately consumes only retained tokens. In particular it does not construct an AST
/// expression, consult a scope, or invoke template/constant evaluation. A declaration without an
/// initializer is a required shell; `none` is accepted only for an optional contract.
pub(crate) fn normalize_source_build_config_contract(
    name: StringId,
    name_location: SourceLocation,
    qualifier: &BuildConfigQualifierSyntax,
    initializer_tokens: &[Token],
    string_table: &mut StringTable,
) -> Result<SourceBuildConfigContract, Box<CompilerDiagnostic>> {
    let name_text = string_table.resolve(name).to_owned();
    let input_name = BuildInputName::new(&name_text).map_err(|_| {
        Box::new(CompilerDiagnostic::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigContractNameInvalid,
            name_location,
        ))
    })?;
    let value_type = build_input_type_from_parsed(&qualifier.type_annotation).ok_or_else(|| {
        Box::new(CompilerDiagnostic::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigQualifierUnsupportedType,
            parsed_type_location(&qualifier.type_annotation),
        ))
    })?;

    let (required, default) = if initializer_tokens.is_empty() {
        (!value_type.is_optional(), None)
    } else if initializer_tokens.len() != 1 {
        return Err(source_default_type_mismatch(
            name,
            value_type,
            "non-primitive",
            initializer_tokens
                .first()
                .map(|token| token.location.clone())
                .unwrap_or_else(|| qualifier.qualifier_location.clone()),
            string_table,
        ));
    } else {
        let token = &initializer_tokens[0];
        match &token.kind {
            TokenKind::NoneLiteral => {
                if !value_type.is_optional() {
                    return Err(source_default_type_mismatch(
                        name,
                        value_type,
                        "None",
                        token.location.clone(),
                        string_table,
                    ));
                }
                (false, None)
            }
            TokenKind::StringSliceLiteral(value) => {
                let value = PrimitiveBuildValue::String(string_table.resolve(*value).to_owned());
                validate_source_default_primitive(
                    name,
                    value_type,
                    value,
                    token.location.clone(),
                    string_table,
                )?
            }
            TokenKind::BoolLiteral(value) => validate_source_default_primitive(
                name,
                value_type,
                PrimitiveBuildValue::Bool(*value),
                token.location.clone(),
                string_table,
            )?,
            TokenKind::CharLiteral(value) => validate_source_default_primitive(
                name,
                value_type,
                PrimitiveBuildValue::Char(*value),
                token.location.clone(),
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
                    Box::new(CompilerDiagnostic::invalid_number_literal(
                        value.source_text,
                        reason,
                        token.location.clone(),
                    ))
                })?;
                validate_source_default_primitive(
                    name,
                    value_type,
                    materialized,
                    token.location.clone(),
                    string_table,
                )?
            }
            _ => {
                return Err(source_default_type_mismatch(
                    name,
                    value_type,
                    "non-primitive",
                    token.location.clone(),
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
        location: qualifier.qualifier_location.clone(),
    })
}

fn validate_source_default_primitive(
    name: StringId,
    value_type: BuildInputType,
    value: PrimitiveBuildValue,
    location: SourceLocation,
    string_table: &mut StringTable,
) -> Result<(bool, Option<PrimitiveBuildValue>), Box<CompilerDiagnostic>> {
    if !value_type.accepts_primitive(value.primitive_type()) {
        return Err(source_default_type_mismatch(
            name,
            value_type,
            value.primitive_type().name(),
            location,
            string_table,
        ));
    }
    Ok((false, Some(value)))
}

fn source_default_type_mismatch(
    name: StringId,
    value_type: BuildInputType,
    provided: &str,
    location: SourceLocation,
    string_table: &mut StringTable,
) -> Box<CompilerDiagnostic> {
    Box::new(CompilerDiagnostic::invalid_config_reason(
        Some(name),
        InvalidConfigReason::ConfigInputTypeMismatch {
            provided: string_table.intern(provided),
            expected: string_table.intern(&build_input_type_name(value_type)),
            provided_argument_index: None,
        },
        location,
    ))
}

/// Find one `#Config` marker in a retained token slice.
///
/// Header preparation uses this flat scan to reject body and nested placements before AST
/// construction. It intentionally reports only the marker location and adjacency; declaration
/// parsing remains the owner of the complete qualifier grammar.
pub(crate) fn find_config_qualifier_marker(
    tokens: &[Token],
    string_table: &StringTable,
) -> Option<(SourceLocation, bool)> {
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

        let on_same_line =
            pair[0].location.end_pos.line_number == pair[1].location.start_pos.line_number;
        let adjacent = on_same_line
            && pair[0].location.end_pos.char_column + 1 == pair[1].location.start_pos.char_column;
        return Some((pair[0].location.clone(), adjacent));
    }

    None
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
/// line boundary after `of` is diagnosed at that authored token rather than being interpreted as
/// an inferred declaration type.
pub(crate) fn parse_build_config_qualifier(
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
) -> Result<BuildConfigQualifierSyntax, Box<CompilerDiagnostic>> {
    let qualifier_location = token_stream.current_location();
    require_config_marker_adjacent(token_stream)?;
    token_stream.advance(); // past `#`

    match token_stream.current_token_kind() {
        TokenKind::Symbol(name) if string_table.resolve(*name) == "Config" => {
            token_stream.advance();
        }
        _ => {
            return Err(Box::new(CompilerDiagnostic::expected_token(
                TokenKind::Symbol(string_table.intern("Config")),
                Some(token_stream.current_token_kind().to_owned()),
                token_stream.current_location(),
            )));
        }
    }

    if token_stream.current_token_kind() != &TokenKind::Of {
        return Err(Box::new(CompilerDiagnostic::expected_token(
            TokenKind::Of,
            Some(token_stream.current_token_kind().to_owned()),
            token_stream.current_location(),
        )));
    }
    token_stream.advance();

    let type_annotation = parse_type_annotation(
        token_stream,
        TypeAnnotationContext::BuildConfigContract,
        string_table,
    )?;

    Ok(BuildConfigQualifierSyntax {
        type_annotation,
        qualifier_location,
        default_none: false,
    })
}

/// `#Config` has a qualifier-specific spacing rule, distinct from ordinary `#` bindings.
fn require_config_marker_adjacent(
    token_stream: &FileTokens,
) -> Result<(), Box<CompilerDiagnostic>> {
    let Some(current_token) = token_stream.tokens.get(token_stream.index) else {
        return Ok(());
    };
    let Some(next_token) = token_stream.tokens.get(token_stream.index + 1) else {
        return Ok(());
    };

    let on_same_line =
        current_token.location.end_pos.line_number == next_token.location.start_pos.line_number;
    let adjacent = on_same_line
        && current_token.location.end_pos.char_column + 1
            == next_token.location.start_pos.char_column;
    if !adjacent {
        return Err(Box::new(CompilerDiagnostic::common_syntax_mistake(
            CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing,
            current_token.location.clone(),
        )));
    }

    Ok(())
}
