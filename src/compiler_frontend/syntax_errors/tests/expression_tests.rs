use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_primary_span, parse_single_file_ast_diagnostic,
};

fn assert_common_syntax_mistake(source: &str, expected_reason_key: &'static str) {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0031", Some(expected_reason_key));
}

#[test]
fn detects_double_equal_as_equality_mistake() {
    assert_common_syntax_mistake(
        "value = 1 == 2\n",
        "common_syntax_mistake.equality_operator",
    );
}

#[test]
fn detects_bang_equal_as_inequality_mistake() {
    assert_common_syntax_mistake(
        "value = 1 != 2\n",
        "common_syntax_mistake.inequality_operator",
    );
}

#[test]
fn detects_and_and_as_conjunction_mistake() {
    assert_common_syntax_mistake(
        "value = true && false\n",
        "common_syntax_mistake.logical_and_operator",
    );
}

#[test]
fn detects_or_or_as_disjunction_mistake() {
    assert_common_syntax_mistake(
        "value = true || false\n",
        "common_syntax_mistake.logical_or_operator",
    );
}

#[test]
fn detects_bang_as_boolean_negation_mistake() {
    assert_common_syntax_mistake(
        "value = !true\n",
        "common_syntax_mistake.boolean_bang_negation",
    );
}

#[test]
fn common_syntax_mistakes_span_the_whole_operator() {
    for (source, reason_key, operator) in [
        (
            "value = 1 == 2\n",
            "common_syntax_mistake.equality_operator",
            "==",
        ),
        (
            "value = 1 === 2\n",
            "common_syntax_mistake.equality_operator",
            "===",
        ),
        (
            "value = 1 != 2\n",
            "common_syntax_mistake.inequality_operator",
            "!=",
        ),
        (
            "value = true && false\n",
            "common_syntax_mistake.logical_and_operator",
            "&&",
        ),
        (
            "value = true || false\n",
            "common_syntax_mistake.logical_or_operator",
            "||",
        ),
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0031", Some(reason_key));
        let operator_start = source.find(operator).expect("operator should be present") as u32;
        let operator_span = LocalSpan::exact(
            operator_start,
            operator.len() as u32,
            &mut ExtendedSpanBuilder::new(),
        )
        .expect("the operator span should fit inline");

        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, operator_span)),
            "{source}",
        );
    }

    // The same tag sequence can also be authored as two separate tokens. Only adjacent source
    // bytes prove the C-family operator, so every case below keeps the ordinary diagnostic for the
    // token the parser stops on instead of claiming the operator the user did not write.
    for (source, expected_code, expected_reason_key, stopped_token) in [
        (
            "value = = 1\n",
            "MOTH-SYNTAX-0031",
            Some("common_syntax_mistake.expression_assignment"),
            "= 1",
        ),
        (
            "value = 1 = = 2\n",
            "MOTH-SYNTAX-0031",
            Some("common_syntax_mistake.expression_assignment"),
            "= =",
        ),
        (
            "value = 1 =\n= 2\n",
            "MOTH-SYNTAX-0031",
            Some("common_syntax_mistake.expression_assignment"),
            "=\n",
        ),
        (
            "value = 1 ! = 2\n",
            "MOTH-RULE-0051",
            Some("invalid_fallible_handling.bang_on_non_fallible"),
            "! =",
        ),
        ("value = true & & false\n", "MOTH-SYNTAX-0002", None, "& &"),
        ("value = true | | false\n", "MOTH-SYNTAX-0002", None, "| |"),
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        let diagnostic = assert_ast_diagnostic(&failure, expected_code, expected_reason_key);
        let token_start = source
            .find(stopped_token)
            .expect("the stopped token should be present") as u32;
        let token_span = LocalSpan::exact(token_start, 1, &mut ExtendedSpanBuilder::new())
            .expect("one byte always fits inline");

        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, token_span)),
            "{source}",
        );
    }
}

/// A single-line source whose `==` starts at `operator_offset` bytes.
///
/// WHAT: pads with one comment line so the operator lands on the requested byte.
/// WHY: the frozen inline span start limit sits at four mebibytes, so these sources force an
/// extended token span. An extended range cannot prove adjacency, so the recogniser must keep the
/// ordinary diagnostic rather than inventing a widened span.
fn source_with_equality_at(operator_offset: u32) -> String {
    let statement = "value = 1 == 2\n";
    let operator_in_statement = statement
        .find("==")
        .expect("the operator is in the statement");
    let prefix_length = operator_offset as usize - operator_in_statement - 1;
    let mut source = String::with_capacity(prefix_length + 1 + statement.len());
    source.push_str("--");
    source.push_str(&" ".repeat(prefix_length - 2));
    source.push('\n');
    source.push_str(statement);
    source
}

#[test]
fn inline_start_boundary_keeps_the_ordinary_equality_diagnostic() {
    let operator_offset = 4 * 1024 * 1024 - 1;
    let source = source_with_equality_at(operator_offset);
    assert_eq!(source.find("=="), Some(operator_offset as usize));

    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0031",
        Some("common_syntax_mistake.expression_assignment"),
    );
    let token_span = LocalSpan::exact(operator_offset, 1, &mut ExtendedSpanBuilder::new())
        .expect("the first operator byte still fits inline");

    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, token_span)),
        "the first `=` is inline and keeps its own one-byte span",
    );
}

#[test]
fn extended_operator_spans_keep_the_ordinary_equality_diagnostic() {
    let operator_offset = 4 * 1024 * 1024 + 11;
    let source = source_with_equality_at(operator_offset);
    assert_eq!(source.find("=="), Some(operator_offset as usize));

    let failure = parse_single_file_ast_diagnostic(&source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0031",
        Some("common_syntax_mistake.expression_assignment"),
    );
    let span = ast_error_primary_span(diagnostic).expect("the equality mistake has a primary span");

    assert_eq!(span.source(), SourceId::COMPILATION_ROOT);
    assert!(
        span.local().is_extended(),
        "an unresolvable operator run keeps the token's own extended span",
    );
}
