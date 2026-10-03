//! Shared fixture contract for the parser-parity corpus.
//!
//! WHAT: owns the fixture, expectation and assertion helpers consumed by the family leaf modules.
//! WHY: later families add a declared leaf module, build a `fixture`, then call `assert_fixture`
//! from one `#[test]` that takes `lock_counter_test()` once.
//!
//! `SourceExpectation::Accept` reuses `MonExpectation::Accept(Value)` for the source matcher.
//! `AcceptObserved(Value)` supplies a distinct source-side tree for documented differences.
//! Rejections name a source diagnostic reason and exact source site. A case whose accept/reject
//! outcomes differ must name its entry in `gaps::PARITY_DIFFERENCES` through
//! `with_outcome_difference`; the assertion then requires the outcomes to differ.
//!
//! MON `Value` has no `Some` wrapper or record nominal identity: optional values and decimals need
//! explicit source expectations because MON omits those parts of the receiving type identity.
//! Attach `with_source_optional`, `with_source_nominal_type`, or `with_source_decimal` where needed.
//! Decimal source expectations name an exact coefficient and scale; they never derive one by
//! converting MON's display text through the compiler's decimal materialiser.
//!
//! Fixtures default to `NumericProfile::STANDARD`. `with_profile` sets the compiler parse profile
//! and MON schema profile together. The matcher compares field names and order, nominal
//! identities, numeric identity and exact payloads. It reads folded `data` constants and typed
//! start-body expressions where source-only forms such as maps are not retained in the
//! folded-value store.

use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompileTimeEvaluationErrorReason, DiagnosticPayload,
    DiagnosticToken, InvalidCallShapeReason, InvalidChoiceVariantReason,
    InvalidControlFlowStatementReason, InvalidExpressionReason, InvalidMapLiteralReason,
    InvalidMapTypeReason, InvalidStringEscapeReason, NameNamespace, ReservedNameOwner,
    TypeMismatchContext,
};
use crate::compiler_frontend::source::ExtendedSpanBuilder;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result_with_profile;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::mon::{MonErrorCode, NumericProfile, PathSegment, Schema, Span, Value, decode_document};
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

use super::gaps::{ParityDifference, parity_difference};
use super::source_observation::{
    SourceIdentityExpectation, SourceIdentityExpectationKind, assert_source_matches,
    observe_data_value,
};

/// One paired compiler-source and MON case.
pub(super) struct ParityFixture {
    name: String,
    literal: String,
    moth_declarations: String,
    source_override: Option<String>,
    schema: Schema,
    profile: NumericProfile,
    source_identity_expectations: Vec<SourceIdentityExpectation>,
    source_optional_paths: Vec<String>,
    expected_source: SourceExpectation,
    expected_mon: MonExpectation,
    outcome_difference: Option<&'static ParityDifference>,
}

/// Independently expected result for the source parser.
pub(super) enum SourceExpectation {
    /// Match the source typed value against the same independent `Value` used by MON.
    Accept,
    /// Use this typed source expectation when source and MON intentionally differ.
    AcceptObserved(Value),
    Reject {
        code: &'static str,
        reason: SourceReason,
        site: SourceSite,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum SourceRegion {
    Declarations,
    Literal,
}

#[derive(Debug)]
pub(super) struct SourceSite {
    region: SourceRegion,
    byte_offset: usize,
    text: String,
}

pub(super) enum SourceReason {
    ReservedName(ReservedNameOwner),
    UnknownName,
    UnknownTypeName,
    UnknownVariant,
    InvalidChoiceVariant(InvalidChoiceVariantReason),
    NamedArgumentNotFound,
    ValueBlockOutsideReceiver,
    CommonSyntaxMistake(CommonSyntaxMistakeReason),
    CompileTimeEvaluation(CompileTimeEvaluationErrorReason),
    InvalidExpression(InvalidExpressionReason),
    ExpectedToken(TokenTag),
    InvalidCharacter(char),
    InvalidCharLiteral,
    InvalidStringEscape(InvalidStringEscapeReason),
    InvalidMapLiteral(InvalidMapLiteralReason),
    DuplicateDeclaration,
    DuplicateMapKey,
    InvalidMapKeyType,
    CallShape(CallShapeExpectation),
    NumberLiteral(NumberLiteralErrorReason),
    TypeMismatch(TypeMismatchContext),
    UnexpectedToken,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CallShapeExpectation {
    ExtraPositional { expected_count: usize },
    DuplicateArgument { parameter_index: usize },
    MissingArgument { parameter_index: usize },
}

pub(super) enum MonExpectation {
    Accept(Value),
    Reject {
        code: MonErrorCode,
        span: Span,
        path: Vec<PathSegment>,
    },
    SchemaRejected {
        code: MonErrorCode,
        path: Vec<PathSegment>,
    },
}

/// Build a fixture with the delivered STANDARD profile on both parser boundaries.
pub(super) fn fixture(
    name: impl Into<String>,
    literal: impl Into<String>,
    moth_declarations: impl Into<String>,
    schema: Schema,
    expected_source: SourceExpectation,
    expected_mon: MonExpectation,
) -> ParityFixture {
    ParityFixture {
        name: name.into(),
        literal: literal.into(),
        moth_declarations: moth_declarations.into(),
        source_override: None,
        schema: schema.with_profile(NumericProfile::STANDARD),
        profile: NumericProfile::STANDARD,
        source_identity_expectations: Vec::new(),
        source_optional_paths: Vec::new(),
        expected_source,
        expected_mon,
        outcome_difference: None,
    }
}

impl ParityFixture {
    /// Select the same numeric contract for source typing and MON schema preparation.
    pub(super) fn with_profile(mut self, profile: NumericProfile) -> Self {
        self.profile = profile;
        self.schema = self.schema.with_profile(profile);
        self
    }

    pub(super) fn with_source_override(mut self, source: impl Into<String>) -> Self {
        self.source_override = Some(source.into());
        self
    }

    /// Expect differing source/MON outcomes owned by one named parity difference.
    pub(super) fn with_outcome_difference(mut self, difference_name: &str) -> Self {
        self.outcome_difference = Some(parity_difference(difference_name));
        self
    }

    /// Supply the nominal source type absent from MON's record-valued struct representation.
    pub(super) fn with_source_nominal_type(
        mut self,
        path: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        self.push_source_identity_expectation(SourceIdentityExpectation {
            path: path.into(),
            kind: SourceIdentityExpectationKind::Nominal(name.into()),
        });
        self
    }

    /// Supply the exact source decimal identity omitted from MON's textual decimal value.
    pub(super) fn with_source_decimal(
        mut self,
        path: impl Into<String>,
        coefficient: impl Into<String>,
        scale: u16,
    ) -> Self {
        self.push_source_identity_expectation(SourceIdentityExpectation {
            path: path.into(),
            kind: SourceIdentityExpectationKind::Decimal {
                coefficient: coefficient.into(),
                scale,
            },
        });
        self
    }

    /// Require the source observer to retain the optional wrapper absent from MON values.
    pub(super) fn with_source_optional(mut self, path: impl Into<String>) -> Self {
        let path = path.into();
        assert!(
            !self.source_optional_paths.contains(&path),
            "fixture '{}' has more than one optional expectation at '{}'",
            self.name,
            path
        );
        self.source_optional_paths.push(path);
        self
    }

    fn push_source_identity_expectation(&mut self, expectation: SourceIdentityExpectation) {
        assert!(
            self.source_identity_expectations
                .iter()
                .all(|existing| existing.path != expectation.path),
            "fixture '{}' has more than one source identity expectation at '{}'",
            self.name,
            expectation.path
        );
        self.source_identity_expectations.push(expectation);
    }
}

fn source_site(region: SourceRegion, source: &str, token: &str, nth: usize) -> SourceSite {
    let occurrence = nth
        .checked_sub(1)
        .expect("source-site occurrence numbers are 1-based");
    let byte_offset = source
        .match_indices(token)
        .nth(occurrence)
        .map(|(offset, _)| offset)
        .unwrap_or_else(|| {
            panic!("expected occurrence {nth} of {token:?} in {region:?} source {source:?}")
        });
    SourceSite {
        region,
        byte_offset,
        text: token.to_owned(),
    }
}

pub(super) fn in_literal(literal: &str, token: &str, nth: usize) -> SourceSite {
    source_site(SourceRegion::Literal, literal, token, nth)
}

pub(super) fn in_declarations(declarations: &str, token: &str, nth: usize) -> SourceSite {
    source_site(SourceRegion::Declarations, declarations, token, nth)
}

pub(super) fn source_reserved_keyword(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-RULE-0039",
        reason: SourceReason::ReservedName(ReservedNameOwner::Keyword),
        site,
    }
}

pub(super) fn source_unknown_name(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-RULE-0034",
        reason: SourceReason::UnknownName,
        site,
    }
}

pub(super) fn source_unknown_type_name(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-RULE-0035",
        reason: SourceReason::UnknownTypeName,
        site,
    }
}

pub(super) fn source_invalid_number(
    reason: NumberLiteralErrorReason,
    site: SourceSite,
) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-SYNTAX-0008",
        reason: SourceReason::NumberLiteral(reason),
        site,
    }
}

pub(super) fn source_type_mismatch(
    context: TypeMismatchContext,
    site: SourceSite,
) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-TYPE-0001",
        reason: SourceReason::TypeMismatch(context),
        site,
    }
}

pub(super) fn source_unknown_variant(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-RULE-0029",
        reason: SourceReason::UnknownVariant,
        site,
    }
}

pub(super) fn source_named_argument_not_found(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-RULE-0054",
        reason: SourceReason::NamedArgumentNotFound,
        site,
    }
}

pub(super) fn source_value_block_outside_receiver(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-RULE-0042",
        reason: SourceReason::ValueBlockOutsideReceiver,
        site,
    }
}

pub(super) fn source_expression_assignment(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-SYNTAX-0031",
        reason: SourceReason::CommonSyntaxMistake(CommonSyntaxMistakeReason::ExpressionAssignment),
        site,
    }
}

pub(super) fn source_invalid_expression(
    reason: InvalidExpressionReason,
    site: SourceSite,
) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-SYNTAX-0023",
        reason: SourceReason::InvalidExpression(reason),
        site,
    }
}

pub(super) fn source_invalid_string_escape(
    reason: InvalidStringEscapeReason,
    site: SourceSite,
) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-SYNTAX-0034",
        reason: SourceReason::InvalidStringEscape(reason),
        site,
    }
}

pub(super) fn source_invalid_map_literal(
    reason: InvalidMapLiteralReason,
    site: SourceSite,
) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-SYNTAX-0033",
        reason: SourceReason::InvalidMapLiteral(reason),
        site,
    }
}

pub(super) fn source_unexpected_token(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-SYNTAX-0002",
        reason: SourceReason::UnexpectedToken,
        site,
    }
}

pub(super) fn source_none_literal_context(site: SourceSite) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-RULE-0053",
        reason: SourceReason::CompileTimeEvaluation(
            CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext,
        ),
        site,
    }
}

pub(super) fn mon_span(literal: &str, token: &str, nth: usize) -> Span {
    let occurrence = nth
        .checked_sub(1)
        .expect("MON span occurrence numbers are 1-based");
    let start = literal
        .match_indices(token)
        .nth(occurrence)
        .map(|(start, _)| start)
        .unwrap_or_else(|| panic!("expected occurrence {nth} of {token:?} in {literal:?}"));
    Span {
        start: start as u32,
        end: (start + token.len()) as u32,
    }
}

pub(super) fn mon_identifier_error(
    literal: &str,
    name: &str,
    path: Vec<PathSegment>,
) -> MonExpectation {
    MonExpectation::Reject {
        code: MonErrorCode::InvalidIdentifier,
        span: mon_span(literal, name, 1),
        path,
    }
}

pub(super) fn mon_named_entry_error(
    literal: &str,
    name: &str,
    path: Vec<PathSegment>,
) -> MonExpectation {
    MonExpectation::Reject {
        code: MonErrorCode::UnexpectedToken,
        span: mon_span(literal, name, 1),
        path,
    }
}

pub(super) fn mon_numeric_error(
    literal: &str,
    token: &str,
    nth: usize,
    code: MonErrorCode,
    path: Vec<PathSegment>,
) -> MonExpectation {
    MonExpectation::Reject {
        code,
        span: mon_span(literal, token, nth),
        path,
    }
}

/// Compare decoded MON values without letting derived `f64` equality erase signed zero.
pub(super) fn assert_mon_value_matches(expected: &Value, actual: &Value, case: &str, path: &str) {
    match (expected, actual) {
        (Value::Float(expected_float), Value::Float(actual_float))
        | (Value::F16(expected_float), Value::F16(actual_float))
        | (Value::F32(expected_float), Value::F32(actual_float))
        | (Value::F64(expected_float), Value::F64(actual_float)) => {
            if expected_float.to_bits() != actual_float.to_bits() {
                mon_value_mismatch(expected, actual, case, path);
            }
        }
        (Value::Record(expected_fields), Value::Record(actual_fields)) => {
            assert_mon_fields_match(expected_fields, actual_fields, case, path);
        }
        (Value::Collection(expected_values), Value::Collection(actual_values)) => {
            if expected_values.len() != actual_values.len() {
                mon_value_mismatch(expected, actual, case, path);
            }
            for (index, (expected_value, actual_value)) in
                expected_values.iter().zip(actual_values).enumerate()
            {
                assert_mon_value_matches(
                    expected_value,
                    actual_value,
                    case,
                    &format!("{path}[{index}]"),
                );
            }
        }
        (Value::Map(expected_entries), Value::Map(actual_entries)) => {
            if expected_entries.len() != actual_entries.len() {
                mon_value_mismatch(expected, actual, case, path);
            }
            for (index, ((expected_key, expected_value), (actual_key, actual_value))) in
                expected_entries.iter().zip(actual_entries).enumerate()
            {
                assert_mon_value_matches(
                    expected_key,
                    actual_key,
                    case,
                    &format!("{path}[key:{index}]"),
                );
                assert_mon_value_matches(
                    expected_value,
                    actual_value,
                    case,
                    &format!("{path}[value:{index}]"),
                );
            }
        }
        (
            Value::Choice {
                qualifier: expected_qualifier,
                variant: expected_variant,
                fields: expected_fields,
            },
            Value::Choice {
                qualifier: actual_qualifier,
                variant: actual_variant,
                fields: actual_fields,
            },
        ) => {
            if expected_qualifier != actual_qualifier || expected_variant != actual_variant {
                mon_value_mismatch(expected, actual, case, path);
            }
            assert_mon_fields_match(expected_fields, actual_fields, case, path);
        }
        _ if expected == actual => {}
        _ => mon_value_mismatch(expected, actual, case, path),
    }
}

fn assert_mon_fields_match(
    expected_fields: &[(String, Value)],
    actual_fields: &[(String, Value)],
    case: &str,
    path: &str,
) {
    if expected_fields.len() != actual_fields.len() {
        panic!(
            "MON value mismatch in case '{case}' at path '{path}': expected fields {expected_fields:?}, observed fields {actual_fields:?}"
        );
    }
    for ((expected_name, expected_value), (actual_name, actual_value)) in
        expected_fields.iter().zip(actual_fields)
    {
        let field_path = if path.is_empty() {
            expected_name.clone()
        } else {
            format!("{path}.{expected_name}")
        };
        if expected_name != actual_name {
            mon_value_mismatch(expected_value, actual_value, case, &field_path);
        }
        assert_mon_value_matches(expected_value, actual_value, case, &field_path);
    }
}

fn mon_value_mismatch(expected: &Value, actual: &Value, case: &str, path: &str) -> ! {
    panic!(
        "MON value mismatch in case '{case}' at path '{path}': expected {expected:?}, observed {actual:?}"
    )
}

/// Source envelope placed between a fixture's declarations and its literal.
const LITERAL_PREFIX: &str = "data #= (";

/// Assert source and MON independently against the fixture's explicit expected outcomes.
pub(super) fn assert_fixture(fixture: ParityFixture) {
    let source = fixture.source_override.clone().unwrap_or_else(|| {
        format!(
            "{}{LITERAL_PREFIX}{})\n",
            fixture.moth_declarations, fixture.literal
        )
    });
    let source_result = parse_single_file_ast_build_result_with_profile(&source, fixture.profile);
    let source_accepted = match source_result {
        Ok((build_result, path_fork, string_table)) => {
            let expected_value = match &fixture.expected_source {
                SourceExpectation::Accept => match &fixture.expected_mon {
                    MonExpectation::Accept(value) => value,
                    MonExpectation::Reject { .. } | MonExpectation::SchemaRejected { .. } => {
                        panic!(
                            "source fixture '{}' accepts without an independent typed value",
                            fixture.name
                        )
                    }
                },
                SourceExpectation::AcceptObserved(value) => value,
                SourceExpectation::Reject { code, .. } => {
                    panic!(
                        "source accepted fixture '{}' despite expected {code}:\n{source}",
                        fixture.name
                    )
                }
            };
            let observed = observe_data_value(&build_result, &path_fork, &string_table)
                .unwrap_or_else(|| {
                    panic!(
                        "source fixture '{}' accepted but had no observable 'data' value:\n{source}",
                        fixture.name
                    )
                });
            assert_source_matches(
                expected_value,
                &observed,
                &fixture.name,
                "",
                &fixture.source_identity_expectations,
                &fixture.source_optional_paths,
            );
            true
        }
        Err(diagnostic) => match fixture.expected_source {
            SourceExpectation::Accept | SourceExpectation::AcceptObserved(_) => {
                panic!(
                    "source rejected fixture '{}' with {} {:?}:\n{source}",
                    fixture.name,
                    diagnostic.identity().code,
                    diagnostic.payload
                )
            }
            SourceExpectation::Reject { code, reason, site } => {
                assert_source_rejection(
                    &diagnostic,
                    code,
                    reason,
                    site,
                    &fixture.name,
                    &fixture.moth_declarations,
                    &source,
                );
                false
            }
        },
    };

    let mon_accepted = match fixture.schema.prepare() {
        Err(error) => match fixture.expected_mon {
            MonExpectation::SchemaRejected { code, path } => {
                assert_eq!(error.code, code, "MON schema error for '{}'", fixture.name);
                assert_eq!(error.span, None, "MON schema span for '{}'", fixture.name);
                assert_eq!(error.path, path, "MON schema path for '{}'", fixture.name);
                false
            }
            MonExpectation::Accept(_) | MonExpectation::Reject { .. } => {
                panic!(
                    "MON schema for '{}' failed preparation with {:?}: {}",
                    fixture.name, error.code, error.detail
                )
            }
        },
        Ok(schema) => {
            let mon_result = decode_document(&fixture.literal, &schema);
            match (mon_result, fixture.expected_mon) {
                (Ok(value), MonExpectation::Accept(expected)) => {
                    assert_mon_value_matches(&expected, &value, &fixture.name, "");
                    true
                }
                (Ok(_), MonExpectation::Reject { code, .. }) => {
                    panic!(
                        "MON accepted fixture '{}' despite expected {:?}: {:?}",
                        fixture.name, code, fixture.literal
                    )
                }
                (Ok(_), MonExpectation::SchemaRejected { code, .. }) => {
                    panic!(
                        "MON schema for '{}' prepared despite expected {:?}",
                        fixture.name, code
                    )
                }
                (Err(error), MonExpectation::Accept(_)) => {
                    panic!(
                        "MON rejected fixture '{}' with {:?} at {:?}: {}",
                        fixture.name, error.code, error.span, error.detail
                    )
                }
                (Err(error), MonExpectation::SchemaRejected { code, .. }) => {
                    panic!(
                        "MON schema for '{}' prepared despite expected {:?}; decoding then failed with {:?}",
                        fixture.name, code, error.code
                    )
                }
                (Err(error), MonExpectation::Reject { code, span, path }) => {
                    assert_eq!(error.code, code, "MON error for '{}'", fixture.name);
                    assert_eq!(
                        error.span,
                        Some(span),
                        "MON span for '{}': {:?}",
                        fixture.name,
                        fixture.literal
                    );
                    assert_eq!(
                        error.path, path,
                        "MON path for '{}': {:?}",
                        fixture.name, fixture.literal
                    );
                    false
                }
            }
        }
    };

    match fixture.outcome_difference {
        None => assert_eq!(
            source_accepted, mon_accepted,
            "independently expected source/MON outcomes diverged for '{}': {:?}",
            fixture.name, fixture.literal
        ),
        Some(difference) => assert_ne!(
            source_accepted, mon_accepted,
            "'{}' expected differing outcomes owned by '{}' ({}), but both sides agreed: {:?}",
            fixture.name, difference.name, difference.capability, fixture.literal
        ),
    }
}

fn assert_source_rejection(
    diagnostic: &crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
    expected_code: &str,
    reason: SourceReason,
    site: SourceSite,
    fixture_name: &str,
    moth_declarations: &str,
    source: &str,
) {
    assert_eq!(
        diagnostic.identity().code,
        expected_code,
        "source diagnostic for '{}' had the wrong category: {:?}",
        fixture_name,
        diagnostic.payload
    );
    match reason {
        SourceReason::ReservedName(expected_owner) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::ReservedNameCollision { reserved_by, .. }
                if *reserved_by == expected_owner
        )),
        SourceReason::UnknownName => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::UnknownName {
                namespace: NameNamespace::Value,
                ..
            }
        )),
        SourceReason::UnknownTypeName => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::UnknownName {
                namespace: NameNamespace::Type,
                ..
            }
        )),
        SourceReason::InvalidChoiceVariant(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidChoiceVariant { reason, .. } if *reason == expected
        )),
        SourceReason::UnknownVariant => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidChoiceVariant {
                reason: InvalidChoiceVariantReason::UnknownVariant,
                ..
            }
        )),
        SourceReason::NamedArgumentNotFound => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidCallShape {
                reason: InvalidCallShapeReason::NamedArgumentNotFound { .. },
                ..
            }
        )),
        SourceReason::ValueBlockOutsideReceiver => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidControlFlowStatement {
                reason: InvalidControlFlowStatementReason::ValueBlockOutsideReceiver
            }
        )),
        SourceReason::CommonSyntaxMistake(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::CommonSyntaxMistake { reason } if reason == &expected
        )),
        SourceReason::CompileTimeEvaluation(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::CompileTimeEvaluationError { reason, .. }
                if reason == &expected
        )),
        SourceReason::InvalidExpression(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidExpression { reason } if reason == &expected
        )),
        SourceReason::ExpectedToken(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::ExpectedToken {
                expected: actual, ..
            } if *actual == DiagnosticToken::from_static_tag(expected)
        )),
        SourceReason::InvalidStringEscape(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidStringEscape { reason } if reason == &expected
        )),
        SourceReason::InvalidMapLiteral(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidMapLiteral { reason, .. } if *reason == expected
        )),
        SourceReason::InvalidCharacter(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidCharacter { character } if *character == expected
        )),
        SourceReason::DuplicateDeclaration => {
            assert!(matches!(
                &diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            ))
        }
        SourceReason::InvalidCharLiteral => {
            assert!(matches!(&diagnostic.payload, DiagnosticPayload::None))
        }
        SourceReason::DuplicateMapKey => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidMapLiteral {
                reason: InvalidMapLiteralReason::DuplicateKnownKey,
                ..
            }
        )),
        SourceReason::InvalidMapKeyType => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidMapType {
                reason: InvalidMapTypeReason::UnsupportedKeyType { .. },
                ..
            }
        )),
        SourceReason::CallShape(expected) => match (&diagnostic.payload, expected) {
            (
                DiagnosticPayload::InvalidCallShape {
                    reason: InvalidCallShapeReason::ExtraPositionalArgument { expected_count },
                    ..
                },
                CallShapeExpectation::ExtraPositional {
                    expected_count: expected,
                },
            ) => assert_eq!(expected_count, &expected),
            (
                DiagnosticPayload::InvalidCallShape {
                    reason:
                        InvalidCallShapeReason::DuplicateArgument {
                            parameter_index, ..
                        },
                    ..
                },
                CallShapeExpectation::DuplicateArgument {
                    parameter_index: expected,
                },
            ) => assert_eq!(parameter_index, &expected),
            (
                DiagnosticPayload::InvalidCallShape {
                    reason:
                        InvalidCallShapeReason::MissingArgument {
                            parameter_index, ..
                        },
                    ..
                },
                CallShapeExpectation::MissingArgument {
                    parameter_index: expected,
                },
            ) => assert_eq!(parameter_index, &expected),
            (payload, expectation) => {
                panic!("source diagnostic did not match {expectation:?}: {payload:?}")
            }
        },
        SourceReason::NumberLiteral(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidNumberLiteral { reason, .. } if *reason == expected
        )),
        SourceReason::TypeMismatch(expected) => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::TypeMismatch { context, .. } if *context == expected
        )),
        SourceReason::UnexpectedToken => assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::UnexpectedToken { .. }
        )),
    }

    let primary_span = diagnostic.primary_span.unwrap_or_else(|| {
        panic!(
            "source diagnostic for '{}' had no primary source location: {:?}",
            fixture_name, diagnostic.payload
        )
    });
    let range = primary_span
        .local()
        .resolve_with(ExtendedSpanBuilder::new().resolver());
    let start = range.start() as usize;
    let end = range.end() as usize;
    let region_start = match site.region {
        SourceRegion::Declarations => 0,
        SourceRegion::Literal => moth_declarations.len() + LITERAL_PREFIX.len(),
    };
    let expected_start = region_start + site.byte_offset;
    let expected_end = expected_start + site.text.len();
    assert_eq!(
        (start, end),
        (expected_start, expected_end),
        "source diagnostic for '{}' pointed at the wrong source site: expected {:?} byte offset {} (absolute range {}..{}) for token {:?}, got {:?}",
        fixture_name,
        site.region,
        site.byte_offset,
        expected_start,
        expected_end,
        site.text,
        range
    );
    assert_eq!(
        source.get(start..end),
        Some(site.text.as_str()),
        "source diagnostic for '{}' did not contain its expected token {:?} at exact span {:?}",
        fixture_name,
        site.text,
        range
    );
}
