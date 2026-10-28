//! Focused duplicate-member tests for the shared `| ... |` signature-member parser.
//!
//! WHAT: verifies that duplicate function parameters, struct fields, choice payload fields
//! and trait-requirement parameters are all rejected by the shared signature-member parser
//! with `DuplicateDeclaration` (`MOTH-RULE-0002`), before any HIR or infrastructure invariant
//! can fire.
//! WHY: the shared parser is the single owner of member-name uniqueness. Function-, struct-
//! and choice-specific duplicate validators would duplicate that ownership.

use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::DuplicateDeclarationView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, DiagnosticCategory};
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::record_body::parse_record_body;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    SignatureMemberContext, parse_function_signature_syntax,
    parse_trait_requirement_signature_syntax,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{LexedSource, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{
    TokenCursor, TokenIndex, TokenTag, TokenizerEntryMode,
};

/// Tokenize `source` and position the stream on the first opening `|` so a wrapper parser
/// (`parse_record_body`, `parse_function_signature_syntax`, ...) can advance past it.
///
fn stream_positioned_at_open_bracket(
    source: &str,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    span_builder: &mut ExtendedSpanBuilder,
) -> LexedSource {
    let source_path = path_fork
        .try_intern_portable_path("test.moth", string_table)
        .expect("test path fits");
    let style_directives = StyleDirectiveRegistry::built_ins();
    tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        string_table,
        path_fork,
        crate::compiler_frontend::source::SourceId::COMPILATION_ROOT,
        span_builder,
    )
    .expect("tokenization should succeed")
}

fn cursor_at_open_bracket(tokens: &LexedSource) -> TokenCursor<'_> {
    let range = tokens
        .tokens
        .full_range()
        .expect("test source must expose a checked token range");
    let mut cursor = tokens
        .tokens
        .cursor(range)
        .expect("test source must expose a canonical cursor");
    let open_index = (0..tokens.tokens.len())
        .find_map(|index| {
            let index = TokenIndex::try_from_index(index)?;
            let token = tokens.tokens.token(index).ok()?;
            (token.tag() == TokenTag::TYPE_PARAMETER_BRACKET).then_some(index)
        })
        .expect("test source must contain an opening `|`");
    cursor
        .set_position(open_index)
        .expect("test cursor must seek to the opening `|`");
    cursor
}

fn duplicate_member_spans(
    token_stream: &LexedSource,
    string_table: &mut StringTable,
    expected_name: &str,
) -> (SourceSpan, SourceSpan) {
    let name = string_table.intern(expected_name);
    let spans = (0..token_stream.tokens.len())
        .filter_map(|index| {
            let index = TokenIndex::try_from_index(index)?;
            let token = token_stream.tokens.token(index).ok()?;
            (token.tag() == TokenTag::SYMBOL && token.string_id() == Some(name))
                .then_some(SourceSpan::new(token_stream.file_id, token.span()))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        spans.len(),
        2,
        "test source should contain exactly two `{expected_name}` members",
    );
    (spans[0], spans[1])
}

fn owner_path(path_fork: &mut PathInternerFork, string_table: &mut StringTable) -> PathId {
    path_fork
        .try_intern_portable_path("test.moth", string_table)
        .expect("test path fits")
}

/// Asserts `error` is the shared-parser duplicate-member diagnostic for `expected_name`:
/// stable `MOTH-RULE-0002` kind, current member primary, first member secondary, and the
/// scope-neutral rendered message.
fn assert_shared_duplicate_diagnostic(
    failure: HeaderParseFailure,
    string_table: &StringTable,
    expected_name: &str,
    expected_first_span: SourceSpan,
    expected_duplicate_span: SourceSpan,
) {
    let draft = match failure {
        HeaderParseFailure::Diagnostic(diagnostic) => diagnostic,
        HeaderParseFailure::Infrastructure(error) => {
            panic!("duplicate-member parser infrastructure failure: {error:?}")
        }
    };
    let diagnostics = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = diagnostics
        .first()
        .expect("duplicate-member diagnostic should compact");
    assert_eq!(
        diagnostic.category(),
        DiagnosticCategory::Rule,
        "duplicate members must use the Rule diagnostic category",
    );
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-RULE-0002",
        "duplicate members must use MOTH-RULE-0002 DuplicateDeclaration",
    );

    let facts = diagnostic
        .family_view::<DuplicateDeclarationView>()
        .expect("duplicate members must retain DuplicateDeclaration facts");
    assert_eq!(string_table.resolve(facts.name), expected_name);

    let primary_span = match diagnostic.primary_site() {
        SiteRef::Local(span) => span,
        site => panic!("duplicate member primary must be local, got {site:?}"),
    };
    assert_eq!(
        primary_span, expected_duplicate_span,
        "duplicate member must be the primary exact source span",
    );
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(
        labels.len(),
        1,
        "only the secondary (first) member label remains",
    );
    let secondary = labels[0];
    let secondary_span = match secondary.site {
        SiteRef::Local(span) => span,
        site => panic!("previous-declaration label must be local, got {site:?}"),
    };
    assert!(secondary.spanned);
    assert_eq!(
        secondary_span, expected_first_span,
        "secondary label must retain the first member's exact source span",
    );
    assert_ne!(
        primary_span, secondary_span,
        "primary and secondary member spans must differ",
    );
    assert!(matches!(
        secondary.message,
        DurableLabelMessage::PreviousDeclaration
    ));

    let render_context = DiagnosticRenderContext::new(string_table);
    let guidance = terminal::format_guidance_view(diagnostic, render_context);
    let expected_fragment = format!(
        "Cannot declare '{expected_name}' because that name is already visible in this scope"
    );
    assert!(
        guidance
            .iter()
            .any(|line| line.contains(&expected_fragment)),
        "expected scope-neutral duplicate message, got {guidance:?}",
    );
}

#[test]
fn duplicate_function_parameters_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        "fn | value I32, value I32 | -> I32 :",
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let function_path = owner_path(&mut path_fork, &mut string_table);
    let mut warnings = DiagnosticBag::new();
    let (expected_first_span, expected_duplicate_span) =
        duplicate_member_spans(&token_stream, &mut string_table, "value");
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    let error = parse_function_signature_syntax(
        &mut declaration_cursor,
        &mut warnings,
        &mut string_table,
        function_path,
        &mut path_fork,
    )
    .expect_err("duplicate function parameters must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(
        error,
        &string_table,
        "value",
        expected_first_span,
        expected_duplicate_span,
    );
}

#[test]
fn duplicate_struct_fields_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        "| value I32, value I32 |",
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let struct_path = owner_path(&mut path_fork, &mut string_table);
    let mut warnings = DiagnosticBag::new();
    let (expected_first_span, expected_duplicate_span) =
        duplicate_member_spans(&token_stream, &mut string_table, "value");
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    let error = parse_record_body(
        &mut declaration_cursor,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::StructField,
        struct_path,
        &mut path_fork,
    )
    .expect_err("duplicate struct fields must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(
        error,
        &string_table,
        "value",
        expected_first_span,
        expected_duplicate_span,
    );
}

#[test]
fn duplicate_choice_payload_fields_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        "| message String, message I32 |",
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let choice_path = owner_path(&mut path_fork, &mut string_table);
    let mut warnings = DiagnosticBag::new();
    let (expected_first_span, expected_duplicate_span) =
        duplicate_member_spans(&token_stream, &mut string_table, "message");
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    let error = parse_record_body(
        &mut declaration_cursor,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::ChoicePayloadField,
        choice_path,
        &mut path_fork,
    )
    .expect_err("duplicate choice payload fields must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(
        error,
        &string_table,
        "message",
        expected_first_span,
        expected_duplicate_span,
    );
}

#[test]
fn duplicate_trait_requirement_parameters_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        "| This, value I32, value I32 | -> I32 ;",
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let method_path = owner_path(&mut path_fork, &mut string_table);
    let mut warnings = DiagnosticBag::new();
    let (expected_first_span, expected_duplicate_span) =
        duplicate_member_spans(&token_stream, &mut string_table, "value");
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    let error = parse_trait_requirement_signature_syntax(
        &mut declaration_cursor,
        &mut warnings,
        &mut string_table,
        method_path,
        &mut path_fork,
    )
    .expect_err("duplicate trait-requirement parameters must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(
        error,
        &string_table,
        "value",
        expected_first_span,
        expected_duplicate_span,
    );
}

#[test]
fn distinct_members_parse_successfully_through_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        "| first I32, second String |",
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let struct_path = owner_path(&mut path_fork, &mut string_table);
    let mut warnings = DiagnosticBag::new();
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    let fields = parse_record_body(
        &mut declaration_cursor,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::StructField,
        struct_path,
        &mut path_fork,
    )
    .expect("distinct member names must parse successfully");

    assert_eq!(fields.len(), 2, "both distinct members must be retained");
    assert_eq!(
        string_table.resolve(
            path_fork
                .component(fields[0].id)
                .expect("first field has a name"),
        ),
        "first",
    );
    assert_eq!(
        string_table.resolve(
            path_fork
                .component(fields[1].id)
                .expect("second field has a name"),
        ),
        "second",
    );
}
