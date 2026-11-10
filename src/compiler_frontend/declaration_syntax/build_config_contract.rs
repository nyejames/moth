//! Declaration-owned syntax for compiler build-configuration contracts.
//!
//! WHAT: records the contract facts of a `$config`-marked compile-time binding shared by header
//! preparation and AST declaration resolution.
//! WHY: `$config` is a source modifier on an ordinary typed declaration, not a type or expression
//! category. Keeping its syntax facts here lets source contracts use one owner without a second
//! declaration grammar.

use crate::compiler_frontend::build_config::{
    BuildInputName, BuildInputType, ContractValueError, DeferredNumericLiteral,
    PrimitiveBuildInputType, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{TokenRef, TokenTag};
use moth_lexical::numeric::grammar::NumericLiteralSign;

/// Contract syntax retained for one `$config`-marked compile-time binding.
///
/// The binding's ordinary declaration owns its type annotation and initializer, and absence is
/// derived by the consuming contract stage: header normalization for selected sources, AST resolution for the config source. This value only preserves the authored
/// `$config` span, so the declaration never carries a second copy of either fact.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BuildConfigContractSyntax {
    /// Authored span of the `$config` directive that marked this binding.
    pub(crate) marker_span: SourceSpan,
}

/// One normalized source-owned `$config` declaration shell.
///
/// The shell is collected while header syntax is prepared, before provider interfaces or AST
/// expression resolution exist. Its default is either one value already materialised at the
/// declared contract type, or the absence marker represented by `None`; no expression tree or
/// provider identity is retained here.
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
/// contract syntax lets source-header preparation and declaration-owned resolution share exactly
/// the same type boundary.
pub(crate) fn build_input_type_from_parsed(parsed: &ParsedTypeRef) -> Option<BuildInputType> {
    let primitive = match parsed {
        ParsedTypeRef::BuiltinString { .. } => PrimitiveBuildInputType::String,
        ParsedTypeRef::BuiltinFixedScalar { scalar, .. } => {
            PrimitiveBuildInputType::from_fixed_scalar(*scalar)?
        }
        ParsedTypeRef::BuiltinBool { .. } => PrimitiveBuildInputType::Bool,
        ParsedTypeRef::BuiltinChar { .. } => PrimitiveBuildInputType::Char,
        ParsedTypeRef::Optional { inner, .. } => {
            let primitive = match inner.as_ref() {
                ParsedTypeRef::BuiltinString { .. } => PrimitiveBuildInputType::String,
                ParsedTypeRef::BuiltinFixedScalar { scalar, .. } => {
                    PrimitiveBuildInputType::from_fixed_scalar(*scalar)?
                }
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
        | ParsedTypeRef::BuiltinString { span, .. }
        | ParsedTypeRef::BuiltinChar { span, .. }
        | ParsedTypeRef::BuiltinFixedScalar { span, .. }
        | ParsedTypeRef::BuiltinNumber { span, .. }
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
    type_annotation: &ParsedTypeRef,
    contract: &BuildConfigContractSyntax,
    token: Option<TokenRef<'_>>,
    string_table: &mut StringTable,
) -> Result<SourceBuildConfigContract, HeaderParseFailure> {
    let name_text = string_table.resolve(name).to_owned();
    let input_name = BuildInputName::new(&name_text).map_err(|_| {
        HeaderParseFailure::Diagnostic(project_config::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigContractNameInvalid,
            Some(name_span),
        ))
    })?;
    let value_type = build_input_type_from_parsed(type_annotation).ok_or_else(|| {
        HeaderParseFailure::Diagnostic(project_config::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigQualifierUnsupportedType,
            parsed_type_span(type_annotation),
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
                                InfrastructureFailure::malformed_token_projection(
                                    "source config string payload",
                                    error,
                                ),
                            )
                        })?
                        .ok_or_else(|| {
                            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                                CompilerStage::HeaderParsing,
                                "source config string token is missing its payload",
                            ))
                        })?;
                    accept_source_default(
                        name,
                        value_type,
                        PrimitiveBuildValue::String(spelling.to_owned()).accept_for(value_type),
                        span,
                        string_table,
                    )
                    .map_err(HeaderParseFailure::Diagnostic)?
                }
                TokenTag::BOOL_LITERAL => {
                    let value = token.bool_value().ok_or_else(|| {
                        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                            CompilerStage::HeaderParsing,
                            "source config bool token has a malformed payload",
                        ))
                    })?;
                    accept_source_default(
                        name,
                        value_type,
                        PrimitiveBuildValue::Bool(value).accept_for(value_type),
                        span,
                        string_table,
                    )
                    .map_err(HeaderParseFailure::Diagnostic)?
                }
                TokenTag::CHAR_LITERAL => {
                    let value = token.char_value().ok_or_else(|| {
                        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                            CompilerStage::HeaderParsing,
                            "source config char token has a malformed payload",
                        ))
                    })?;
                    accept_source_default(
                        name,
                        value_type,
                        PrimitiveBuildValue::Char(value).accept_for(value_type),
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
                                InfrastructureFailure::malformed_token_projection(
                                    "source config numeric payload",
                                    error,
                                ),
                            )
                        })?
                        .ok_or_else(|| {
                            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                                CompilerStage::HeaderParsing,
                                "source config numeric token is missing its payload",
                            ))
                        })?;
                    // A numeric default materialises here, at the declared contract's exact scalar.
                    // The literal kind decides which scalar families it can initialise, and digits
                    // outside that scalar's range are reported at the literal span.
                    let literal = DeferredNumericLiteral::new(
                        string_table.resolve(numeric.source_text).to_owned(),
                        string_table.resolve(numeric.normalized_text).to_owned(),
                        matches!(numeric.sign, NumericLiteralSign::Negative),
                        numeric.kind,
                    );
                    accept_source_default(
                        name,
                        value_type,
                        literal.materialise_for(value_type),
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
        span: contract.marker_span,
    })
}

/// Reject a source contract initializer that contains more than one token.
///
/// The source contract stores only one literal default. A longer initializer is
/// therefore a diagnosed non-primitive default rather than a truncated first-token value.
pub(crate) fn normalize_source_build_config_contract_non_primitive(
    name: StringId,
    name_span: SourceSpan,
    type_annotation: &ParsedTypeRef,
    span: SourceSpan,
    string_table: &mut StringTable,
) -> Result<SourceBuildConfigContract, HeaderParseFailure> {
    let name_text = string_table.resolve(name).to_owned();
    BuildInputName::new(&name_text).map_err(|_| {
        HeaderParseFailure::Diagnostic(project_config::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigContractNameInvalid,
            Some(name_span),
        ))
    })?;
    let value_type = build_input_type_from_parsed(type_annotation).ok_or_else(|| {
        HeaderParseFailure::Diagnostic(project_config::invalid_config_reason(
            Some(name),
            InvalidConfigReason::ConfigQualifierUnsupportedType,
            parsed_type_span(type_annotation),
        ))
    })?;
    Err(HeaderParseFailure::Diagnostic(
        source_default_type_mismatch(name, value_type, "non-primitive", Some(span), string_table),
    ))
}

/// Accept one header default at its declared contract, or report why that contract rejects it.
fn accept_source_default(
    name: StringId,
    value_type: BuildInputType,
    value: Result<PrimitiveBuildValue, ContractValueError>,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> Result<(bool, Option<PrimitiveBuildValue>), DiagnosticDraft> {
    value
        .map(|value| (false, Some(value)))
        .map_err(|error| source_default_failure(name, value_type, error, span, string_table))
}

/// Map one contract rejection of a header default onto its user diagnostic.
fn source_default_failure(
    name: StringId,
    value_type: BuildInputType,
    error: ContractValueError,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> DiagnosticDraft {
    match error {
        ContractValueError::TypeMismatch { provided } => {
            source_default_type_mismatch(name, value_type, provided.name(), span, string_table)
        }
        ContractValueError::LiteralOutOfRange { text, reason } => {
            token_lexical::invalid_number_literal(string_table.intern(&text), reason, span)
        }
    }
}

fn source_default_type_mismatch(
    name: StringId,
    value_type: BuildInputType,
    provided: &str,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> DiagnosticDraft {
    project_config::invalid_config_reason(
        Some(name),
        InvalidConfigReason::ConfigInputTypeMismatch {
            provided: string_table.intern(provided),
            expected: string_table.intern(&build_input_type_name(value_type)),
            provided_argument_index: None,
        },
        span,
    )
}
