use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::projects::html_project::styles::code::{
    CodeLanguage, code_formatter, highlight_code_html,
};

#[test]
fn generic_code_highlighter_marks_syntax_but_not_keywords() {
    let highlighted = highlight_code_html("loop(x + 1)", CodeLanguage::Generic);

    assert!(highlighted.contains("<span class='moth-code-parenthesis'>(</span>"));
    assert!(highlighted.contains("<span class='moth-code-operator'>+</span>"));
    assert!(highlighted.contains("<span class='moth-code-number'>1</span>"));
    assert!(!highlighted.contains("moth-code-keyword"));
    assert!(highlighted.contains("loop"));
}

#[test]
fn direct_moth_highlighter_marks_comments_and_keywords() {
    let highlighted = highlight_code_html("loop x\n-- hi", CodeLanguage::Moth);

    assert!(highlighted.contains("<span class='moth-code-keyword'>loop</span>"));
    assert!(highlighted.contains("<span class='moth-code-comment'>-- hi</span>"));
}

#[test]
fn direct_javascript_highlighter_marks_line_comments() {
    let highlighted = highlight_code_html("const x = 1\n// hi", CodeLanguage::JavaScript);

    assert!(highlighted.contains("<span class='moth-code-keyword'>const</span>"));
    assert!(highlighted.contains("<span class='moth-code-comment'>// hi</span>"));
}

#[test]
fn direct_python_highlighter_marks_hash_comments() {
    let highlighted = highlight_code_html("def run():\n# hi", CodeLanguage::Python);

    assert!(highlighted.contains("<span class='moth-code-keyword'>def</span>"));
    assert!(highlighted.contains("<span class='moth-code-comment'># hi</span>"));
}

#[test]
fn direct_typescript_highlighter_marks_type_keywords() {
    let highlighted = highlight_code_html("type Name = string", CodeLanguage::TypeScript);

    assert!(highlighted.contains("<span class='moth-code-keyword'>type</span>"));
    assert!(highlighted.contains("<span class='moth-code-type'>string</span>"));
}

#[test]
fn direct_code_highlighter_preserves_trailing_words_at_eof() {
    let highlighted = highlight_code_html("value", CodeLanguage::Generic);

    assert!(highlighted.ends_with("value"));
}

#[test]
fn direct_code_highlighter_preserves_single_quoted_strings() {
    let highlighted = highlight_code_html("'value'", CodeLanguage::Generic);

    assert!(highlighted.contains("&#39;value&#39;"));
}

#[test]
fn direct_code_highlighter_escapes_html_sensitive_content() {
    let highlighted = highlight_code_html("<tag>", CodeLanguage::Generic);

    assert!(highlighted.contains("&lt;"));
    assert!(highlighted.contains("tag"));
    assert!(highlighted.contains("&gt;"));
    assert!(!highlighted.contains("<tag>"));
}

#[test]
fn text_code_formatter_escapes_html_without_highlighting() {
    let mut string_table = StringTable::new();
    let formatter = code_formatter(CodeLanguage::Text);

    let id = string_table.intern("<tag>&\"quoted\"");
    let input = FormatterInput {
        pieces: vec![FormatterInputPiece::Text(FormatterTextPiece {
            text: id,
            location: SourceLocation::default(),
        })],
    };

    let output = formatter
        .formatter
        .format(input, &mut string_table)
        .expect("code formatter should succeed");
    let content = match &output.output.pieces[0] {
        FormatterOutputPiece::Text(t) => t,
        _ => panic!("Expected text output"),
    };

    assert!(content.starts_with("<code class='codeblock'>"));
    assert!(content.ends_with("</code>"));
    assert!(content.contains("&lt;tag&gt;&amp;&quot;quoted&quot;"));
    assert!(!content.contains("<tag>"));
    assert!(!content.contains("moth-code-"));
}
