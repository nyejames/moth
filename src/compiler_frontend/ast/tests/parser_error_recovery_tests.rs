//! Parser and ordinary-body recovery regression tests.
//!
//! WHAT: asserts that malformed inputs fail with stable typed diagnostic facts, that independent
//!       ordinary bodies keep reporting after a user error within bounded budgets, and that failed
//!       bodies leave no state a later body can observe.
//! WHY: parser and emitter recovery changes must not degrade fail-fast paths, skip independent
//!      errors or publish broken construction state.

use crate::compiler_frontend::compiler_errors::{CompilationOutcome, CompilationResult};
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidMultiBindCode, InvalidMultiBindView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    InvalidMatchPatternView, InvalidStatementPositionView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    InvalidFunctionSignatureCode, InvalidFunctionSignatureView, InvalidThisUsageCode,
    InvalidThisUsageView, InvalidTraitKeywordUsageView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    UnclosedSourceScopeView, UnexpectedTokenShape, UnexpectedTokenView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticDiscoveryStops, DiagnosticSeverity, InvalidMatchPatternReason,
    InvalidStatementPositionReason, InvalidTraitKeywordUsageReason, SourceScopeConstruct,
};
use crate::compiler_frontend::module_compilation::AstRecoveryPolicy;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::parse_support::{
    LayoutSite, assert_ast_diagnostic, assert_ast_unclosed_source_scope, ast_error_label_spans,
    ast_error_primary_span, first_ast_error_view, parse_single_file_ast,
    parse_single_file_ast_diagnostic, parse_single_file_ast_diagnostic_with_policy,
    source_end_span,
};
use std::num::NonZeroU32;

fn assert_failure(source: &str, code: &str, reason_key: &str) -> CompilationResult<()> {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, code, Some(reason_key));
    failure
}

struct InvalidMatchPatternCase {
    name: &'static str,
    source: &'static str,
    expected_reason: InvalidMatchPatternReason,
}

struct MultiBindUnsupportedRhsCase {
    name: &'static str,
    source: &'static str,
}

struct ReservedTraitKeywordCase {
    name: &'static str,
    source: &'static str,
    expected_reason: InvalidTraitKeywordUsageReason,
}

#[test]
fn reports_missing_signature_colon() {
    let failure = assert_failure(
        "f|| -> I32\n;\n",
        "MOTH-RULE-0062",
        "invalid_function_signature.missing_colon_after_returns",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidFunctionSignatureView>()
        .expect("missing signature colon should retain typed facts");
    assert_eq!(
        facts.code,
        InvalidFunctionSignatureCode::MissingColonAfterReturns
    );
}

#[test]
fn reports_stray_comma_in_function_body() {
    let failure = assert_failure(
        "broken||:\n    value = 1\n    ,\n;\n",
        "MOTH-SYNTAX-0030",
        "invalid_statement_position.unexpected_comma",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidStatementPositionView>()
        .expect("stray comma should retain typed facts");
    assert_eq!(
        facts.reason,
        InvalidStatementPositionReason::UnexpectedComma
    );
}

#[test]
fn reports_invalid_match_pattern_forms_as_permanent_rule_errors() {
    let cases = [
        InvalidMatchPatternCase {
            name: "scalar wildcard arm",
            source: "value = 1\nif value is:\n    _ => io.line([: [\"one\"]])\n;\n",
            expected_reason: InvalidMatchPatternReason::WildcardNotSupported,
        },
        InvalidMatchPatternCase {
            name: "choice payload wildcard capture",
            source: "Result :: Ok, Err | message String |;\n\nresult = Result::Err(\"bad\")\nif result is:\n    Err(_) => io.line([: [\"err\"]])\n;\n",
            expected_reason: InvalidMatchPatternReason::WildcardNotSupported,
        },
    ];

    for case in cases {
        let failure = assert_failure(
            case.source,
            "MOTH-RULE-0049",
            "invalid_match_pattern.wildcard_not_supported",
        );
        let diagnostic = first_ast_error_view(&failure);
        let facts = diagnostic
            .family_view::<InvalidMatchPatternView>()
            .expect("invalid match pattern should retain typed facts");
        assert_eq!(facts.reason, case.expected_reason, "{}", case.name);
        assert_eq!(facts.variant_name, None, "{}", case.name);
        assert_eq!(facts.scrutinee_name, None, "{}", case.name);
    }
}

struct UnclosedSourceScopeCase {
    name: &'static str,
    source: &'static str,
    construct: SourceScopeConstruct,
    reason_key: &'static str,
    opener: LayoutSite<'static>,
}

#[test]
fn reports_unclosed_source_scopes_with_their_construct() {
    let cases = [
        UnclosedSourceScopeCase {
            name: "entry if",
            source: "ready ~= false\nif ready:\n    io.line(\"hidden\")\n",
            construct: SourceScopeConstruct::IfBlock,
            reason_key: "unclosed_source_scope.if_block",
            opener: LayoutSite::first(":"),
        },
        UnclosedSourceScopeCase {
            name: "entry loop",
            source: "count ~= 0\nloop count < 3:\n    ~count += 1\n",
            construct: SourceScopeConstruct::LoopBlock,
            reason_key: "unclosed_source_scope.loop_block",
            opener: LayoutSite::first(":"),
        },
        UnclosedSourceScopeCase {
            name: "entry catch",
            source: "values ~= {1, 2}\n~values.set(0, 8) catch:\n",
            construct: SourceScopeConstruct::CatchHandler,
            reason_key: "unclosed_source_scope.catch_handler",
            opener: LayoutSite::first(":"),
        },
        UnclosedSourceScopeCase {
            name: "match",
            source: "value = 1\nif value is:\n    1 => io.line(\"one\")\n",
            construct: SourceScopeConstruct::MatchBlock,
            reason_key: "unclosed_source_scope.match_block",
            opener: LayoutSite::first(":"),
        },
        UnclosedSourceScopeCase {
            name: "value-producing if",
            source: "ready ~= true\nvalue = if ready:\n    then 1\n",
            construct: SourceScopeConstruct::IfBlock,
            reason_key: "unclosed_source_scope.if_block",
            opener: LayoutSite::first(":"),
        },
        UnclosedSourceScopeCase {
            name: "value-producing match",
            source: "value = 1\nresult = if value is:\n    1 => then \"one\"\n",
            construct: SourceScopeConstruct::MatchBlock,
            reason_key: "unclosed_source_scope.match_block",
            opener: LayoutSite::first(":"),
        },
    ];

    for case in cases {
        let failure = parse_single_file_ast_diagnostic(case.source);
        assert_ast_unclosed_source_scope(
            &failure,
            case.source,
            case.name,
            case.construct,
            case.reason_key,
            case.opener,
        );
    }
}

#[test]
fn labels_the_child_closer_that_left_an_entry_scope_open() {
    let source = "ready ~= false\nif ready:\n    if ready:\n        io.line(\"x\")\n    ;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0042",
        Some("unclosed_source_scope.if_block"),
    );
    let facts = diagnostic
        .family_view::<UnclosedSourceScopeView>()
        .expect("the unclosed outer if should retain its typed construct");
    assert_eq!(facts.reason, SourceScopeConstruct::IfBlock);
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(source_end_span(source))
    );
    assert_eq!(
        ast_error_label_spans(diagnostic),
        vec![
            Some(LayoutSite::first(":").span_in(source)),
            Some(LayoutSite::first(";").span_in(source)),
        ],
        "the report should label the outer opener and the `;` that closed its child"
    );
}

#[test]
fn labels_the_child_closer_after_a_later_closerless_statement() {
    let source = "ready ~= false\nvalues ~= {1, 2}\n~values.set(0, 8) catch:\n    if ready:\n        io.line(\"x\")\n    ;\n    io.line(\"after\")\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0042",
        Some("unclosed_source_scope.catch_handler"),
    );
    let facts = diagnostic
        .family_view::<UnclosedSourceScopeView>()
        .expect("the unclosed catch should retain its typed construct");
    assert_eq!(facts.reason, SourceScopeConstruct::CatchHandler);
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(source_end_span(source))
    );
    assert_eq!(
        ast_error_label_spans(diagnostic),
        vec![
            Some(LayoutSite::first(":").span_in(source)),
            Some(LayoutSite::first(";").span_in(source)),
        ],
        "a later closerless statement must not discard the recorded child closer"
    );
}

#[test]
fn labels_the_last_child_closer_before_match_eof() {
    let source = "value = 1\nif value is:\n    1 =>\n        if true:\n            io.line(\"x\")\n        ;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0042",
        Some("unclosed_source_scope.match_block"),
    );
    let facts = diagnostic
        .family_view::<UnclosedSourceScopeView>()
        .expect("the unclosed match should retain its typed construct");
    assert_eq!(facts.reason, SourceScopeConstruct::MatchBlock);
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(source_end_span(source))
    );
    assert_eq!(
        ast_error_label_spans(diagnostic),
        vec![
            Some(LayoutSite::first(":").span_in(source)),
            Some(LayoutSite::first(";").span_in(source)),
        ],
        "the report should label the match opener and the `;` that closed its arm's child"
    );
}

#[test]
fn labels_the_last_child_closer_from_an_else_body() {
    let source = "ready ~= true\nif ready:\n    if ready:\n        io.line(\"x\")\n    ;\nelse\n    io.line(\"y\")\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0042",
        Some("unclosed_source_scope.if_block"),
    );
    let facts = diagnostic
        .family_view::<UnclosedSourceScopeView>()
        .expect("the unclosed if should retain its typed construct");
    assert_eq!(facts.reason, SourceScopeConstruct::IfBlock);
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(source_end_span(source))
    );
    assert_eq!(
        ast_error_label_spans(diagnostic),
        vec![
            Some(LayoutSite::first(":").span_in(source)),
            Some(LayoutSite::first(";").span_in(source)),
        ],
        "the else-body report should label the `;` that closed the earlier branch's child"
    );
}

#[test]
fn reports_multi_bind_malformed_comma_sequence() {
    let failure = assert_failure(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\na, , b = pair()\n",
        "MOTH-SYNTAX-0002",
        "invalid_multi_bind.missing_target_after_comma",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<UnexpectedTokenView>()
        .expect("malformed multi-bind should retain unexpected-token facts");
    assert!(matches!(
        facts.shape,
        UnexpectedTokenShape::MultiBind {
            code: InvalidMultiBindCode::MissingTargetAfterComma,
            target_name: None,
            ..
        }
    ));
}

#[test]
fn reports_string_literal_as_invalid_multi_bind_target() {
    let failure = assert_failure(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\na, \"b\" = pair()\n",
        "MOTH-SYNTAX-0002",
        "invalid_multi_bind.missing_target_after_comma",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<UnexpectedTokenView>()
        .expect("a string multi-bind target retains unexpected-token facts");
    assert!(matches!(
        facts.shape,
        UnexpectedTokenShape::MultiBind {
            code: InvalidMultiBindCode::MissingTargetAfterComma,
            target_name: None,
            ..
        }
    ));
}

#[test]
fn parses_multi_bind_targets_across_comma_continuation_newline() {
    parse_single_file_ast("pair || -> I32, I32:\n    return 1, 2\n;\n\na,\n    b = pair()\n");
}

#[test]
fn reports_multi_bind_trailing_comma_at_the_comma() {
    let failure = assert_failure(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\na,\n    = pair()\n",
        "MOTH-SYNTAX-0002",
        "invalid_multi_bind.missing_target_after_comma",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<UnexpectedTokenView>()
        .expect("trailing multi-bind comma should retain unexpected-token facts");
    assert!(matches!(
        facts.shape,
        UnexpectedTokenShape::MultiBind {
            code: InvalidMultiBindCode::MissingTargetAfterComma,
            target_name: None,
            ..
        }
    ));
}

#[test]
fn reports_multi_bind_mutable_target_without_explicit_type() {
    let failure = assert_failure(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\na, b ~= pair()\n",
        "MOTH-RULE-0045",
        "invalid_multi_bind.mutable_target_needs_explicit_type",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidMultiBindView>()
        .expect("invalid multi-bind should retain typed facts");
    assert_eq!(
        facts.reason,
        InvalidMultiBindCode::MutableTargetNeedsExplicitType
    );
    assert!(facts.target_name.is_some());
}

#[test]
fn reports_multi_bind_with_unsupported_rhs_forms_rejected() {
    let cases = [
        MultiBindUnsupportedRhsCase {
            name: "variable",
            source: "value ~= 1\na, b = value\n",
        },
        MultiBindUnsupportedRhsCase {
            name: "literal",
            source: "a, b = 1\n",
        },
        MultiBindUnsupportedRhsCase {
            name: "field access",
            source: "Thing = |\n    x I32,\n    y I32,\n|\nthing ~= Thing(1, 2)\na, b = thing.x\n",
        },
    ];

    for case in cases {
        let failure = assert_failure(
            case.source,
            "MOTH-RULE-0045",
            "invalid_multi_bind.unsupported_rhs",
        );
        let diagnostic = first_ast_error_view(&failure);
        let facts = diagnostic
            .family_view::<InvalidMultiBindView>()
            .expect("invalid multi-bind should retain typed facts");
        assert_eq!(
            facts.reason,
            InvalidMultiBindCode::UnsupportedRhs,
            "{}",
            case.name
        );
        assert_eq!(facts.target_name, None, "{}", case.name);
    }
}

#[test]
fn reports_reserved_trait_keywords_outside_trait_syntax() {
    let cases = [
        ReservedTraitKeywordCase {
            name: "must binding",
            source: "must = 1\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "This statement",
            source: "f||:\n    This\n;\n",
            expected_reason: InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must expression",
            source: "value = must\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "This expression",
            source: "value = This\n",
            expected_reason: InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must copy target",
            source: "value = copy must\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must signature parameter",
            source: "sum|must I32| -> I32:\n    return 1\n;\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must postfix member",
            source: "Point = |\n    value I32 = 1,\n|\n\npoint ~= Point()\nvalue = point.must\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
    ];

    for case in cases {
        let reason_key = match &case.expected_reason {
            InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax => {
                "invalid_trait_keyword_usage.must_outside_trait_syntax"
            }
            InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax => {
                "invalid_trait_keyword_usage.this_outside_trait_syntax"
            }
        };
        let failure = assert_failure(case.source, "MOTH-RULE-0076", reason_key);
        let diagnostic = first_ast_error_view(&failure);
        let facts = diagnostic
            .family_view::<InvalidTraitKeywordUsageView>()
            .expect("trait keyword diagnostic should retain typed facts");
        assert_eq!(facts.reason, case.expected_reason, "{}", case.name);
    }
}

#[test]
fn reports_trait_this_keyword_outside_trait_declaration_type_position() {
    let failure = assert_failure(
        "value This = 1\n",
        "MOTH-RULE-0040",
        "invalid_this_usage.outside_trait_declaration",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidThisUsageView>()
        .expect("invalid this usage should retain typed facts");
    assert_eq!(facts.code, InvalidThisUsageCode::OutsideTraitDeclaration);
}

/// A token allowance large enough that only the module count limit can stop a fixture.
const GENEROUS_WORK_ALLOWANCE: u32 = 1_000_000;

fn recovery_policy(module_error_limit: Option<u32>, work_allowance: u32) -> AstRecoveryPolicy {
    AstRecoveryPolicy {
        module_error_allowance: module_error_limit
            .map(|limit| NonZeroU32::new(limit).expect("test limits are non-zero")),
        body_work_allowance: work_allowance,
    }
}

fn site_span(site: SiteRef<'_>) -> Option<SourceSpan> {
    match site {
        SiteRef::Absent => None,
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
    }
}

fn diagnosed_reports(result: &CompilationResult<()>) -> &DiagnosticReportSet {
    match result {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        other => panic!("expected a diagnosed AST result, found {other:?}"),
    }
}

fn error_count(result: &CompilationResult<()>) -> usize {
    diagnosed_reports(result)
        .iter_severity(DiagnosticSeverity::Error)
        .count()
}

/// The exact authored span of the first `needle`, stamped like the fixture's token spans.
fn expected_source_span(source: &str, needle: &str) -> SourceSpan {
    let start = source
        .find(needle)
        .unwrap_or_else(|| panic!("fixture source should contain {needle:?}"));
    expected_source_span_at(start, needle)
}

/// The exact authored span of `needle` after `marker`, for names a fixture repeats.
fn expected_source_span_after(source: &str, marker: &str, needle: &str) -> SourceSpan {
    let marker_start = source
        .find(marker)
        .unwrap_or_else(|| panic!("fixture source should contain {marker:?}"));
    let start = marker_start
        + source[marker_start..]
            .find(needle)
            .unwrap_or_else(|| panic!("fixture source after {marker:?} should contain {needle:?}"));
    expected_source_span_at(start, needle)
}

fn expected_source_span_at(start: usize, needle: &str) -> SourceSpan {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start as u32, needle.len() as u32, &mut builder)
        .expect("fixture spans fit an inline test span");
    SourceSpan::new(SourceId::COMPILATION_ROOT, local)
}

/// Asserts the complete ordered primary-error set as stable codes with authored anchors.
#[track_caller]
fn assert_primary_errors(result: &CompilationResult<()>, expected: &[(&str, SourceSpan)]) {
    let errors: Vec<(&'static str, Option<SourceSpan>)> = diagnosed_reports(result)
        .iter_severity(DiagnosticSeverity::Error)
        .map(|(_, diagnostic): (_, DiagnosticView<'_>)| {
            (
                diagnostic.descriptor().code,
                site_span(diagnostic.primary_site()),
            )
        })
        .collect();

    assert_eq!(
        errors.len(),
        expected.len(),
        "retained primary errors should be exactly {expected:?}, found {errors:?}"
    );
    for ((code, span), (expected_code, expected_span)) in errors.iter().zip(expected) {
        assert_eq!(code, expected_code, "diagnostic order and codes");
        assert_eq!(
            *span,
            Some(*expected_span),
            "{expected_code} primary anchor"
        );
    }

    assert_eq!(
        diagnosed_reports(result)
            .iter_severity(DiagnosticSeverity::Warning)
            .count(),
        0,
        "a failed module retains no warnings"
    );
}

#[test]
fn diagnostic_recovery_independent_bodies() {
    let source = "\
take |value I32| -> I32:
    return value
;

first || -> I32:
    return missing
;

second || -> I32:
    return take(\"text\")
;

valid || -> I32:
    return 1
;
";
    let result = parse_single_file_ast_diagnostic(source);

    // A failed body must not hide an independent error in a later body, and its own failure must
    // not cascade into the call that supplies a well-formed signature.
    assert_primary_errors(
        &result,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            ("MOTH-TYPE-0001", expected_source_span(source, "\"text\"")),
        ],
    );
}

#[test]
fn diagnostic_recovery_failed_callee_signature() {
    let source = "\
callee |value I32| -> I32:
    return missing
;

caller || -> I32:
    return callee(\"text\")
;

catching || -> I32:
    value = callee(1) catch then 0
    return value
;
";
    let result = parse_single_file_ast_diagnostic(source);

    // The failed callee still has a usable signature, so the caller's argument error is
    // independent. The failed body never installs an infallibility proof, so `catch` on the
    // callee produces no follow-on diagnostic.
    assert_primary_errors(
        &result,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            ("MOTH-TYPE-0001", expected_source_span(source, "\"text\"")),
        ],
    );
}

// Module file-value resources are not exercised here because the AST harness has no module
// origin. Resource interning is idempotent and append-only, so a failed body leaves no entry that a
// later body can misread.
#[test]
fn diagnostic_recovery_shared_state_isolation() {
    let source = "\
identity type T |value T| -> T:
    return value
;

probe || -> String:
    fallback U64 = 0
    value ~= 1 + 2 catch then fallback
    ~value = 4294967296
    text = [:[value]]
    ok = identity(\"x\")
    return text + missing
;

later || -> String:
    ok = identity(\"ok\")
    return [:[value]]
;
";
    let result = parse_single_file_ast_diagnostic(source);

    // `value` only existed as a failed body's local mutable binding. A stale lookup would resolve
    // it and hide the second error; a generic ticket collision or changed signature would fail
    // the call.
    assert_primary_errors(
        &result,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            (
                "MOTH-RULE-0034",
                expected_source_span_after(source, "later ||", "value"),
            ),
        ],
    );
}

/// One retained simple body: the newline after the signature colon, `return`, one value token and
/// the newline before its standalone closer.
///
/// The header stage captures the body range from the first token after the signature colon up to
/// the closing delimiter, so these fixture bodies charge exactly four work tokens.
const SIMPLE_BODY_WORK_TOKENS: u32 = 4;

#[test]
fn diagnostic_recovery_work_boundary_admits_an_exact_fit() {
    let source = "\
first || -> I32:
    return missing
;
second || -> I32:
    return also_missing
;
third || -> I32:
    return third_missing
;
";
    // The first failed body is uncharged, so admitting both later bodies costs 4 plus 4.
    let exact_fit_allowance = SIMPLE_BODY_WORK_TOKENS * 2;
    let (exact_fit, exact_stops) = parse_single_file_ast_diagnostic_with_policy(
        source,
        recovery_policy(None, exact_fit_allowance),
    );
    assert_eq!(error_count(&exact_fit), 3);
    assert_primary_errors(
        &exact_fit,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            (
                "MOTH-RULE-0034",
                expected_source_span(source, "also_missing"),
            ),
            (
                "MOTH-RULE-0034",
                expected_source_span(source, "third_missing"),
            ),
        ],
    );
    assert_eq!(exact_stops, DiagnosticDiscoveryStops::NONE);

    let (one_short, short_stops) = parse_single_file_ast_diagnostic_with_policy(
        source,
        recovery_policy(None, exact_fit_allowance - 1),
    );
    assert_eq!(error_count(&one_short), 2);
    assert_primary_errors(
        &one_short,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            (
                "MOTH-RULE-0034",
                expected_source_span(source, "also_missing"),
            ),
        ],
    );
    assert!(short_stops.contains(DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD));
}

#[test]
fn diagnostic_recovery_work_boundary_overflow_stops_before_the_next_body() {
    let source = "\
first || -> I32:
    return missing
;
second || -> I32:
    return also_missing
;
third || -> I32:
    return 1 + missing_third
;
";
    // The third body retains six tokens: the newline after its colon, `return`, `1`, `+`,
    // `missing_third` and the newline before its standalone closer.
    const LONG_BODY_WORK_TOKENS: u32 = 6;
    let allowance = SIMPLE_BODY_WORK_TOKENS + LONG_BODY_WORK_TOKENS;

    // After the second body, fewer tokens remain than the third body needs.
    let (short, short_stops) =
        parse_single_file_ast_diagnostic_with_policy(source, recovery_policy(None, allowance - 1));
    assert_eq!(
        error_count(&short),
        2,
        "the work guard refuses the third body before it parses"
    );
    assert_primary_errors(
        &short,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            (
                "MOTH-RULE-0034",
                expected_source_span(source, "also_missing"),
            ),
        ],
    );
    assert_eq!(
        short_stops,
        DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD
    );

    // The full sum admits the longer third body and therefore its diagnostic.
    let (full, full_stops) =
        parse_single_file_ast_diagnostic_with_policy(source, recovery_policy(None, allowance));
    assert_eq!(error_count(&full), 3);
    assert_eq!(full_stops, DiagnosticDiscoveryStops::NONE);
}

#[test]
fn diagnostic_recovery_work_boundary_successful_body_consumes_allowance() {
    let source = "\
first || -> I32:
    return missing
;
second || -> I32:
    return 2
;
third || -> I32:
    return third_missing
;
";
    let full_allowance = SIMPLE_BODY_WORK_TOKENS * 2;

    // The second body is valid but still spends its retained tokens, so only the full cumulative
    // allowance admits the third body.
    let (full, full_stops) =
        parse_single_file_ast_diagnostic_with_policy(source, recovery_policy(None, full_allowance));
    assert_eq!(error_count(&full), 2);
    assert_primary_errors(
        &full,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            (
                "MOTH-RULE-0034",
                expected_source_span(source, "third_missing"),
            ),
        ],
    );
    assert_eq!(full_stops, DiagnosticDiscoveryStops::NONE);

    let (one_short, short_stops) = parse_single_file_ast_diagnostic_with_policy(
        source,
        recovery_policy(None, full_allowance - 1),
    );
    assert_eq!(
        error_count(&one_short),
        1,
        "an uncharged or refunded successful body would admit the third body"
    );
    assert!(short_stops.contains(DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD));
}

#[test]
fn diagnostic_recovery_scope_boundary_generic_template() {
    let source = "\
first || -> I32:
    return missing
;
identity type T |value T| -> T:
    return also_missing
;
";
    let result = parse_single_file_ast_diagnostic(source);

    // Generic template validation is its own fail-fast boundary. Recovery ends before it, so its
    // body error is not discovered.
    assert_primary_errors(
        &result,
        &[("MOTH-RULE-0034", expected_source_span(source, "missing"))],
    );
}

#[test]
fn diagnostic_recovery_scope_boundary_implicit_start() {
    let source = "\
first || -> I32:
    return missing
;
value = also_missing
";
    let result = parse_single_file_ast_diagnostic(source);

    // The entry header is emitted last and keeps its own fail-fast boundary.
    assert_primary_errors(
        &result,
        &[("MOTH-RULE-0034", expected_source_span(source, "missing"))],
    );
}

#[test]
fn diagnostic_recovery_scope_boundary_const_template() {
    let source = "\
first || -> I32:
    return missing
;
[: [also_missing]]
";
    let result = parse_single_file_ast_diagnostic(source);

    // Const-template parsing and folding are their own fail-fast boundary.
    assert_primary_errors(
        &result,
        &[("MOTH-RULE-0034", expected_source_span(source, "missing"))],
    );
}

#[test]
fn diagnostic_recovery_module_allowance_one_matches_fail_fast() {
    let source = "\
first || -> I32:
    return missing
;
second || -> I32:
    return also_missing
;
third || -> I32:
    return third_missing
;
";
    let (result, stops) = parse_single_file_ast_diagnostic_with_policy(
        source,
        recovery_policy(Some(1), GENEROUS_WORK_ALLOWANCE),
    );

    assert_primary_errors(
        &result,
        &[("MOTH-RULE-0034", expected_source_span(source, "missing"))],
    );
    assert_eq!(stops, DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE);
}

#[test]
fn diagnostic_recovery_module_allowance_two_retains_two_and_stops() {
    let source = "\
first || -> I32:
    return missing
;
second || -> I32:
    return also_missing
;
third || -> I32:
    return third_missing
;
";
    let (result, stops) = parse_single_file_ast_diagnostic_with_policy(
        source,
        recovery_policy(Some(2), GENEROUS_WORK_ALLOWANCE),
    );

    assert_primary_errors(
        &result,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            (
                "MOTH-RULE-0034",
                expected_source_span(source, "also_missing"),
            ),
        ],
    );
    assert_eq!(stops, DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE);
}

#[test]
fn diagnostic_recovery_module_allowance_reached_on_last_body_sets_no_stop() {
    let source = "\
first || -> I32:
    return missing
;
second || -> I32:
    return also_missing
;
";
    let (result, stops) = parse_single_file_ast_diagnostic_with_policy(
        source,
        recovery_policy(Some(2), GENEROUS_WORK_ALLOWANCE),
    );

    assert_primary_errors(
        &result,
        &[
            ("MOTH-RULE-0034", expected_source_span(source, "missing")),
            (
                "MOTH-RULE-0034",
                expected_source_span(source, "also_missing"),
            ),
        ],
    );
    assert_eq!(
        stops,
        DiagnosticDiscoveryStops::NONE,
        "reaching the allowance on the last available body skips no work"
    );
}
