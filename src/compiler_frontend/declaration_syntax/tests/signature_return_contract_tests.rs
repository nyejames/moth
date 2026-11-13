//! Whole `-> !` return-contract syntax tests for the shared signature parser.
//!
//! WHAT: verifies that a bare `!` immediately after `->` becomes `Never` for both a function
//!       body signature and a bodyless trait requirement, preserving the `!` source span.
//! WHY: the signature shell is the only syntax owner allowed to consume a bare `!`; every later
//!       layer must receive the whole-contract enum instead of re-deriving Never from slots.

use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    InvalidFunctionSignatureCode, InvalidFunctionSignatureView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionReturnContractSyntax, FunctionSignatureSyntax,
    parse_function_signature_syntax_with_evaluation, parse_trait_requirement_signature_syntax,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::ExtendedSpanBuilder;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{LexedSource, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{
    TokenCursor, TokenIndex, TokenTag, TokenizerEntryMode,
};

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

fn parse_function_contract(source: &str) -> FunctionSignatureSyntax {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        source,
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let function_path: PathId = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let mut warnings = DiagnosticBag::new();
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    parse_function_signature_syntax_with_evaluation(
        &mut declaration_cursor,
        &mut warnings,
        &mut string_table,
        function_path,
        &mut path_fork,
        CallableEvaluation::Runtime,
    )
    .expect("the signature should parse")
}

fn parse_trait_contract(source: &str) -> FunctionSignatureSyntax {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        source,
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let method_path: PathId = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let mut warnings = DiagnosticBag::new();
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    parse_trait_requirement_signature_syntax(
        &mut declaration_cursor,
        &mut warnings,
        &mut string_table,
        method_path,
        &mut path_fork,
    )
    .expect("the trait requirement signature should parse")
}

#[test]
fn function_never_return_contract_is_a_whole_contract() {
    let signature = parse_function_contract("fn || -> ! :");

    assert!(signature.parameters.is_empty());
    assert!(
        matches!(
            signature.return_contract,
            FunctionReturnContractSyntax::Never
        ),
        "`-> !` must not parse as an ordinary return list: {:?}",
        signature.return_contract
    );
}

#[test]
fn trait_requirement_never_return_contract_is_parsed_without_a_body() {
    let signature = parse_trait_contract("fn || -> !");

    assert!(signature.parameters.is_empty());
    assert!(
        matches!(
            signature.return_contract,
            FunctionReturnContractSyntax::Never
        ),
        "`-> !` must not parse as an ordinary trait return list: {:?}",
        signature.return_contract
    );
}

#[test]
fn ordinary_function_returns_remain_an_ordinary_contract() {
    let signature = parse_function_contract("fn || -> String :");

    let FunctionReturnContractSyntax::Returns(return_slots) = signature.return_contract else {
        panic!("an authored type after `->` must remain an ordinary return contract");
    };
    assert_eq!(return_slots.len(), 1);
}

#[test]
fn no_arrow_function_has_an_empty_ordinary_contract() {
    let signature = parse_function_contract("fn || :");

    let FunctionReturnContractSyntax::Returns(return_slots) = signature.return_contract else {
        panic!("a signature without `->` must be an ordinary no-value contract");
    };
    assert!(return_slots.is_empty());
}

/// Malformed combinations are reported by the shared signature diagnostic family.
fn assert_never_cannot_combine(failure: HeaderParseFailure) {
    let draft = match failure {
        HeaderParseFailure::Diagnostic(diagnostic) => diagnostic,
        HeaderParseFailure::Infrastructure(error) => {
            panic!("malformed Never syntax must be a source diagnostic, got {error:?}")
        }
    };
    let diagnostics = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = diagnostics
        .first()
        .expect("the malformed Never signature should compact one diagnostic");
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-RULE-0062",
        "malformed Never syntax must use the invalid function signature family",
    );
    let facts = diagnostic
        .family_view::<InvalidFunctionSignatureView>()
        .expect("malformed Never syntax must retain its signature facts");
    assert_eq!(
        facts.code,
        InvalidFunctionSignatureCode::NeverCannotCombine,
        "malformed Never syntax must report the targeted combination reason",
    );
}

#[test]
fn function_never_with_following_type_is_rejected() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        "fn || -> ! I32 :",
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let function_path: PathId = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let mut warnings = DiagnosticBag::new();
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    let failure = parse_function_signature_syntax_with_evaluation(
        &mut declaration_cursor,
        &mut warnings,
        &mut string_table,
        function_path,
        &mut path_fork,
        CallableEvaluation::Runtime,
    )
    .expect_err("a type after `-> !` must be rejected");
    assert_never_cannot_combine(failure);
}

#[test]
fn trait_never_with_optional_suffix_is_rejected() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = stream_positioned_at_open_bracket(
        "fn || -> !?",
        &mut path_fork,
        &mut string_table,
        &mut span_builder,
    );
    let method_path: PathId = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let mut warnings = DiagnosticBag::new();
    let mut declaration_cursor = DeclarationCursor::new(cursor_at_open_bracket(&token_stream));
    let failure = parse_trait_requirement_signature_syntax(
        &mut declaration_cursor,
        &mut warnings,
        &mut string_table,
        method_path,
        &mut path_fork,
    )
    .expect_err("an optional suffix after `-> !` must be rejected");
    assert_never_cannot_combine(failure);
}
