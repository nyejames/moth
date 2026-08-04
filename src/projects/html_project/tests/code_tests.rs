use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::projects::html_project::styles::code::{
    CodeLanguage, code_formatter, highlight_code_html,
};
use crate::projects::html_project::styles::escape_html::escape_html_formatter;

#[test]
fn generic_code_highlighter_marks_syntax_but_not_keywords() {
    let highlighted = highlight_code_html("loop(x + 1)", CodeLanguage::Generic);

    assert!(highlighted.contains("<span class='moth-code-delimiter'>(</span>"));
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

#[test]
fn moth_highlighter_uses_compiler_word_classes() {
    let highlighted = highlight_code_html(
        "if import return assert is not and or Int Float Bool String Char None True False async yield checked block",
        CodeLanguage::Moth,
    );

    for word in [
        "if", "import", "return", "assert", "async", "yield", "checked", "block",
    ] {
        assert!(
            highlighted.contains(&format!("<span class='moth-code-keyword'>{word}</span>")),
            "expected keyword span for {word:?} in: {highlighted}"
        );
    }

    for word in ["is", "not", "and", "or"] {
        assert!(
            highlighted.contains(&format!("<span class='moth-code-operator'>{word}</span>")),
            "expected operator span for {word:?} in: {highlighted}"
        );
    }

    for word in [
        "Int", "Float", "Bool", "String", "Char", "None", "True", "False",
    ] {
        assert!(
            highlighted.contains(&format!("<span class='moth-code-type'>{word}</span>")),
            "expected type span for {word:?} in: {highlighted}"
        );
    }
}

#[test]
fn moth_highlighter_wraps_literals_and_keeps_planned_words_plain() {
    let highlighted =
        highlight_code_html("true false none in fn group into where", CodeLanguage::Moth);

    assert_eq!(
        highlighted,
        "<span class='moth-code-literal'>true</span> <span class='moth-code-literal'>false</span> <span class='moth-code-literal'>none</span> in fn group into where"
    );
}

#[test]
fn moth_compound_operators_use_one_span() {
    let highlighted = highlight_code_html(
        "a //= b -> c :: d .. e => f << g >> h <= i >= j // k",
        CodeLanguage::Moth,
    );

    for operator in ["//=", "::", "..", "//"] {
        assert!(
            highlighted.contains(&format!(
                "<span class='moth-code-operator'>{operator}</span>"
            )),
            "expected one operator span for {operator:?} in: {highlighted}"
        );
    }

    for (operator, escaped) in [
        ("->", "-&gt;"),
        ("=>", "=&gt;"),
        ("<<", "&lt;&lt;"),
        (">>", "&gt;&gt;"),
        ("<=", "&lt;="),
        (">=", "&gt;="),
    ] {
        assert!(
            highlighted.contains(&format!(
                "<span class='moth-code-operator'>{escaped}</span>"
            )),
            "expected one escaped operator span for {operator:?} in: {highlighted}"
        );
    }

    assert_eq!(
        highlighted
            .matches("<span class='moth-code-operator'>")
            .count(),
        10,
        "expected exactly one span per compound operator in: {highlighted}"
    );
}

#[test]
fn moth_operator_fallbacks_keep_single_char_spans() {
    let highlighted = highlight_code_html(
        "a = b + c - d * e / f % g ^ h < i > j ~ k # l $ m ! n ? o & p @ q",
        CodeLanguage::Moth,
    );

    for operator in [
        "=", "+", "-", "*", "/", "%", "^", "~", "#", "$", "!", "?", "@",
    ] {
        assert!(
            highlighted.contains(&format!(
                "<span class='moth-code-operator'>{operator}</span>"
            )),
            "expected operator span for {operator:?} in: {highlighted}"
        );
    }

    for (operator, escaped) in [("<", "&lt;"), (">", "&gt;"), ("&", "&amp;")] {
        assert!(
            highlighted.contains(&format!(
                "<span class='moth-code-operator'>{escaped}</span>"
            )),
            "expected escaped operator span for {operator:?} in: {highlighted}"
        );
    }
}

#[test]
fn moth_scanner_does_not_invent_equality_or_logical_operators() {
    let equality = highlight_code_html("a == b", CodeLanguage::Moth);
    assert!(
        equality.contains(
            "<span class='moth-code-operator'>=</span><span class='moth-code-operator'>=</span>"
        ),
        "== must stay two separate '=' operator spans, got: {equality}"
    );

    let inequality = highlight_code_html("a != b", CodeLanguage::Moth);
    assert!(
        inequality.contains(
            "<span class='moth-code-operator'>!</span><span class='moth-code-operator'>=</span>"
        ),
        "!= must stay separate operator spans, got: {inequality}"
    );

    let logical_and = highlight_code_html("a && b", CodeLanguage::Moth);
    assert!(
        logical_and.contains(
            "<span class='moth-code-operator'>&amp;</span><span class='moth-code-operator'>&amp;</span>"
        ),
        "&& must stay two '&' operator spans, got: {logical_and}"
    );

    let logical_or = highlight_code_html("a || b", CodeLanguage::Moth);
    assert!(
        logical_or.contains(
            "<span class='moth-code-delimiter'>|</span><span class='moth-code-delimiter'>|</span>"
        ),
        "|| must stay two delimiter spans, got: {logical_or}"
    );
    assert!(!logical_or.contains("moth-code-operator'>|"));
}

#[test]
fn moth_punctuation_forms_word_boundaries() {
    assert_eq!(
        highlight_code_html("String,", CodeLanguage::Moth),
        "<span class='moth-code-type'>String</span>,"
    );

    assert_eq!(
        highlight_code_html("Status::Ready,", CodeLanguage::Moth),
        "<span class='moth-code-nominal'>Status</span><span class='moth-code-operator'>::</span><span class='moth-code-nominal'>Ready</span>,"
    );

    assert_eq!(
        highlight_code_html("io.line", CodeLanguage::Moth),
        "<span class='moth-code-type'>io</span>.line"
    );
}

#[test]
fn moth_number_runs_cover_exponents_and_separators() {
    for source in ["1", "1_000", "1.5", "1e6", "1e-6", "1.0e+21"] {
        let highlighted = highlight_code_html(source, CodeLanguage::Moth);
        assert_eq!(
            highlighted,
            format!("<span class='moth-code-number'>{source}</span>"),
            "expected one number run for {source:?}"
        );
    }
}

#[test]
fn moth_number_runs_do_not_swallow_range_operators() {
    let highlighted = highlight_code_html("1..10", CodeLanguage::Moth);

    assert_eq!(
        highlighted,
        "<span class='moth-code-number'>1</span><span class='moth-code-operator'>..</span><span class='moth-code-number'>10</span>"
    );
}

#[test]
fn unicode_identifiers_and_text_survive_scanning() {
    assert_eq!(highlight_code_html("héllo", CodeLanguage::Moth), "héllo");
    assert_eq!(
        highlight_code_html("π_value", CodeLanguage::Moth),
        "π_value"
    );
    assert_eq!(highlight_code_html("名前", CodeLanguage::Moth), "名前");

    assert_eq!(
        highlight_code_html("'λ'", CodeLanguage::Moth),
        "<span class='moth-code-string'>&#39;λ&#39;</span>"
    );
    assert_eq!(
        highlight_code_html("\"héllo\"", CodeLanguage::Moth),
        "<span class='moth-code-string'>&quot;héllo&quot;</span>"
    );
}

#[test]
fn escaping_is_preserved_across_plain_string_and_comment_runs() {
    let plain = highlight_code_html("<tag>", CodeLanguage::Moth);
    assert_eq!(
        plain,
        "<span class='moth-code-operator'>&lt;</span>tag<span class='moth-code-operator'>&gt;</span>"
    );

    let string = highlight_code_html("\"<&>\"", CodeLanguage::Moth);
    assert_eq!(
        string,
        "<span class='moth-code-string'>&quot;&lt;&amp;&gt;&quot;</span>"
    );

    let comment = highlight_code_html("-- <&>", CodeLanguage::Moth);
    assert_eq!(
        comment,
        "<span class='moth-code-comment'>-- &lt;&amp;&gt;</span>"
    );
}

#[test]
fn scanner_handles_end_of_input_runs() {
    assert_eq!(highlight_code_html("value", CodeLanguage::Moth), "value");

    assert_eq!(
        highlight_code_html("123", CodeLanguage::Moth),
        "<span class='moth-code-number'>123</span>"
    );

    assert_eq!(
        highlight_code_html("-- comment", CodeLanguage::Moth),
        "<span class='moth-code-comment'>-- comment</span>"
    );

    assert_eq!(
        highlight_code_html("\"unterminated", CodeLanguage::Moth),
        "<span class='moth-code-string'>&quot;unterminated</span>"
    );
}

#[test]
fn rust_profile_keeps_a_regression_example() {
    let highlighted = highlight_code_html(
        "fn main() -> String { let name = \"moth\"; }",
        CodeLanguage::Rust,
    );

    assert!(highlighted.contains("<span class='moth-code-keyword'>fn</span>"));
    assert!(highlighted.contains("<span class='moth-code-operator'>-</span>"));
    assert!(highlighted.contains("<span class='moth-code-keyword'>let</span>"));
    assert!(highlighted.contains("<span class='moth-code-string'>&quot;moth&quot;</span>"));
    assert!(!highlighted.contains("<span class='moth-code-keyword'>main</span>"));
}

#[test]
fn shell_profile_keeps_a_regression_example() {
    let highlighted = highlight_code_html("if true; then echo hi; fi", CodeLanguage::Shell);

    assert!(highlighted.contains("<span class='moth-code-keyword'>if</span>"));
    assert!(highlighted.contains("<span class='moth-code-keyword'>then</span>"));
    assert!(highlighted.contains("<span class='moth-code-keyword'>fi</span>"));
    assert!(highlighted.contains("<span class='moth-code-literal'>true</span>"));
    assert!(highlighted.contains(";"));
}

#[test]
fn moth_highlighter_marks_nominal_names() {
    let highlighted = highlight_code_html("Label Point Maybe Status A", CodeLanguage::Moth);

    for word in ["Label", "Point", "Maybe", "Status", "A"] {
        assert!(
            highlighted.contains(&format!("<span class='moth-code-nominal'>{word}</span>")),
            "expected nominal span for {word:?} in: {highlighted}"
        );
    }
}

#[test]
fn moth_highlighter_marks_error_and_io_namespace_as_types() {
    let highlighted = highlight_code_html("Error io.line io", CodeLanguage::Moth);

    assert!(highlighted.contains("<span class='moth-code-type'>Error</span>"));
    assert!(highlighted.contains("<span class='moth-code-type'>io</span>.line"));
    assert!(!highlighted.contains("moth-code-type'>io</span> <"));
}

#[test]
fn moth_highlighter_marks_functions_declarations_calls_and_methods() {
    let declaration = highlight_code_html("render |title String| -> String:", CodeLanguage::Moth);
    assert!(
        declaration.contains("<span class='moth-code-function'>render</span>"),
        "declaration name before | must be a function, got: {declaration}"
    );

    let call = highlight_code_html("draw_rect(canvas, 10, 20)", CodeLanguage::Moth);
    assert!(
        call.contains("<span class='moth-code-function'>draw_rect</span>"),
        "call name before ( must be a function, got: {call}"
    );

    let method = highlight_code_html("value.render()", CodeLanguage::Moth);
    assert!(
        method.contains("<span class='moth-code-function'>render</span>"),
        "method name before ( must be a function, got: {method}"
    );

    let plain = highlight_code_html("count = total", CodeLanguage::Moth);
    assert_eq!(
        plain,
        "count <span class='moth-code-operator'>=</span> total"
    );
}

#[test]
fn moth_highlighter_marks_contracts_with_bounded_context() {
    let declaration = highlight_code_html("DISPLAY_TEXT must:", CodeLanguage::Moth);
    assert!(
        declaration.contains("<span class='moth-code-contract'>DISPLAY_TEXT</span>"),
        "trait declaration name must be a contract, got: {declaration}"
    );

    let conformance = highlight_code_html("Label must DISPLAY_TEXT", CodeLanguage::Moth);
    assert!(
        conformance.contains("<span class='moth-code-contract'>DISPLAY_TEXT</span>"),
        "conformance trait must be a contract, got: {conformance}"
    );

    let list = highlight_code_html("type A is SERIALIZABLE and COMPARABLE", CodeLanguage::Moth);
    assert!(
        list.contains("<span class='moth-code-contract'>SERIALIZABLE</span>"),
        "generic bound trait must be a contract, got: {list}"
    );
    assert!(
        list.contains("<span class='moth-code-contract'>COMPARABLE</span>"),
        "and-continued trait must be a contract, got: {list}"
    );

    let incompatibility = highlight_code_html("Color must not FROZEN", CodeLanguage::Moth);
    assert!(
        incompatibility.contains("<span class='moth-code-contract'>FROZEN</span>"),
        "must-not trait must be a contract, got: {incompatibility}"
    );
}

#[test]
fn moth_highlighter_keeps_all_caps_constants_non_contract() {
    let highlighted = highlight_code_html("PI TAU E MAX_SIZE", CodeLanguage::Moth);

    assert_eq!(
        highlighted,
        "PI TAU <span class='moth-code-nominal'>E</span> MAX_SIZE"
    );
    assert!(!highlighted.contains("moth-code-contract"));
}

#[test]
fn moth_highlighter_marks_directives_and_import_paths() {
    let directives = highlight_code_html(
        "$md $code $slot $insert $children $html $css",
        CodeLanguage::Moth,
    );
    for directive in [
        "$md",
        "$code",
        "$slot",
        "$insert",
        "$children",
        "$html",
        "$css",
    ] {
        assert!(
            directives.contains(&format!(
                "<span class='moth-code-directive'>{directive}</span>"
            )),
            "expected directive span for {directive:?} in: {directives}"
        );
    }

    let paths = highlight_code_html("import @core/io {print}", CodeLanguage::Moth);
    assert!(
        paths.contains("<span class='moth-code-string'>@core/io</span>"),
        "import path must use the string role, got: {paths}"
    );

    let double_at = highlight_code_html("@@name", CodeLanguage::Moth);
    assert_eq!(
        double_at,
        "<span class='moth-code-operator'>@</span><span class='moth-code-operator'>@</span>name",
        "doubled @ must stay visible as two operator spans, got: {double_at}"
    );
}

#[test]
fn moth_highlighter_keeps_non_directive_dollar_forms_as_operators() {
    let type_dollar = highlight_code_html("$Int", CodeLanguage::Moth);
    assert!(
        type_dollar.contains("<span class='moth-code-operator'>$</span>"),
        "$Int must keep $ as an operator, got: {type_dollar}"
    );
    assert!(
        type_dollar.contains("<span class='moth-code-type'>Int</span>"),
        "Int after $ must keep its type role, got: {type_dollar}"
    );

    let assign = highlight_code_html("$=", CodeLanguage::Moth);
    assert!(
        assign.contains("<span class='moth-code-operator'>$=</span>"),
        "$= must stay one operator span, got: {assign}"
    );
}

#[test]
fn moth_highlighter_keeps_attached_bang_keywords_together() {
    let highlighted = highlight_code_html("return! value cast! value", CodeLanguage::Moth);

    assert!(
        highlighted.contains("<span class='moth-code-keyword'>return!</span>"),
        "return! must be one keyword span, got: {highlighted}"
    );
    assert!(
        highlighted.contains("<span class='moth-code-keyword'>cast!</span>"),
        "cast! must be one keyword span, got: {highlighted}"
    );
}

#[test]
fn moth_highlighter_marks_delimiters() {
    let highlighted = highlight_code_html("a : b ; c | d ( e ) [ f ] { g }", CodeLanguage::Moth);

    for delimiter in [":", ";", "|", "(", ")", "[", "]", "{", "}"] {
        assert!(
            highlighted.contains(&format!(
                "<span class='moth-code-delimiter'>{delimiter}</span>"
            )),
            "expected delimiter span for {delimiter:?} in: {highlighted}"
        );
    }
}

#[test]
fn non_moth_profiles_share_literal_role() {
    let javascript = highlight_code_html("const x = true", CodeLanguage::JavaScript);
    assert!(javascript.contains("<span class='moth-code-literal'>true</span>"));

    let python = highlight_code_html("value = None", CodeLanguage::Python);
    assert!(python.contains("<span class='moth-code-literal'>None</span>"));

    let rust = highlight_code_html("let x = false", CodeLanguage::Rust);
    assert!(rust.contains("<span class='moth-code-literal'>false</span>"));
}

#[test]
fn non_moth_profiles_share_function_and_contract_roles() {
    let javascript = highlight_code_html("function render() {}", CodeLanguage::JavaScript);
    assert!(javascript.contains("<span class='moth-code-function'>render</span>"));

    let python = highlight_code_html("def run():", CodeLanguage::Python);
    assert!(python.contains("<span class='moth-code-function'>run</span>"));

    let rust = highlight_code_html("fn main() {}", CodeLanguage::Rust);
    assert!(rust.contains("<span class='moth-code-function'>main</span>"));

    let typescript =
        highlight_code_html("interface User { name: string }", CodeLanguage::TypeScript);
    assert!(typescript.contains("<span class='moth-code-contract'>User</span>"));

    let rust_trait = highlight_code_html("trait Display {}", CodeLanguage::Rust);
    assert!(rust_trait.contains("<span class='moth-code-contract'>Display</span>"));
}

#[test]
fn rust_types_no_longer_inherit_typescript_words() {
    let rust = highlight_code_html("let value: string = name", CodeLanguage::Rust);
    assert!(
        !rust.contains("<span class='moth-code-type'>string</span>"),
        "Rust must not inherit the TypeScript type word, got: {rust}"
    );

    let typescript = highlight_code_html("let value: string = name", CodeLanguage::TypeScript);
    assert!(typescript.contains("<span class='moth-code-type'>string</span>"));
}

#[test]
fn moth_compound_assignment_forms_use_one_span() {
    let highlighted = highlight_code_html(
        "a += b -= c *= d /= e %= f ^= g #= h ~= i $= j",
        CodeLanguage::Moth,
    );

    for operator in ["+=", "-=", "*=", "/=", "%=", "^=", "#=", "~=", "$="] {
        assert!(
            highlighted.contains(&format!(
                "<span class='moth-code-operator'>{operator}</span>"
            )),
            "expected one operator span for {operator:?} in: {highlighted}"
        );
    }

    assert_eq!(
        highlighted
            .matches("<span class='moth-code-operator'>")
            .count(),
        9,
        "expected exactly one span per compound assignment in: {highlighted}"
    );
}

#[test]
fn moth_reactive_marker_is_not_a_directive() {
    let highlighted = highlight_code_html("$(count)", CodeLanguage::Moth);

    assert!(highlighted.contains("<span class='moth-code-operator'>$</span>"));
    assert!(highlighted.contains("<span class='moth-code-delimiter'>(</span>"));
    assert!(!highlighted.contains("moth-code-directive"));
}

#[test]
fn moth_contract_state_resets_at_operator_boundaries() {
    let arrow = highlight_code_html("TRAIT must ONE -> NEXT", CodeLanguage::Moth);
    assert!(
        arrow.contains("<span class='moth-code-contract'>ONE</span>"),
        "ONE must stay a contract before the arrow, got: {arrow}"
    );
    assert!(
        !arrow.contains("<span class='moth-code-contract'>NEXT</span>"),
        "NEXT must not be a contract after ->, got: {arrow}"
    );

    let assign = highlight_code_html("TRAIT must ONE = NEXT", CodeLanguage::Moth);
    assert!(
        !assign.contains("<span class='moth-code-contract'>NEXT</span>"),
        "NEXT must not be a contract after =, got: {assign}"
    );
}

#[test]
fn moth_directive_and_path_emit_exactly_once() {
    let directive = highlight_code_html("$md", CodeLanguage::Moth);
    assert_eq!(
        directive, "<span class='moth-code-directive'>$md</span>",
        "a directive must emit exactly one span, got: {directive}"
    );

    let path = highlight_code_html("@core/io", CodeLanguage::Moth);
    assert_eq!(
        path, "<span class='moth-code-string'>@core/io</span>",
        "a path must emit exactly one span, got: {path}"
    );

    let import = highlight_code_html("import @core/io", CodeLanguage::Moth);
    assert_eq!(
        import,
        "<span class='moth-code-keyword'>import</span> <span class='moth-code-string'>@core/io</span>",
        "import plus path must emit each token once, got: {import}"
    );
}

#[test]
fn highlighted_output_preserves_every_source_byte_exactly_once() {
    let cases = [
        "value",
        "héllo π_value 名前",
        "-- comment <&>",
        "\"quoted <&>\" and 'single'",
        "$md($slot)",
        "import @core/io {print, line}",
        "a //= b += c .. d :: e -> f => g",
        "return! cast! value",
        "Label must DISPLAY_TEXT",
        "value 42\n-- note",
    ];

    for source in cases {
        let highlighted = highlight_code_html(source, CodeLanguage::Moth);
        let stripped = strip_role_spans(&highlighted);
        let expected = escape_source_for_comparison(source);
        assert_eq!(
            stripped, expected,
            "span-free output must equal escaped input for {source:?}\ngot: {stripped}\nhighlighted: {highlighted}"
        );
    }
}

#[test]
fn escape_html_formatter_covers_all_special_chars_and_unicode() {
    let mut string_table = StringTable::new();
    let formatter = escape_html_formatter();

    let id = string_table.intern("<tag>&\"quoted\" 'single' héllo π");
    let input = FormatterInput {
        pieces: vec![FormatterInputPiece::Text(FormatterTextPiece {
            text: id,
            location: SourceLocation::default(),
        })],
    };

    let output = formatter
        .formatter
        .format(input, &mut string_table)
        .expect("escape formatter should succeed");
    let content = match &output.output.pieces[0] {
        FormatterOutputPiece::Text(text) => text,
        _ => panic!("Expected text output"),
    };

    assert_eq!(
        content,
        "&lt;tag&gt;&amp;&quot;quoted&quot; &#39;single&#39; héllo π"
    );
}

#[test]
fn non_moth_declaration_roles_target_only_the_exact_next_identifier() {
    let anonymous = highlight_code_html("function (value) {}", CodeLanguage::JavaScript);
    assert_eq!(
        anonymous,
        "<span class='moth-code-keyword'>function</span> <span class='moth-code-delimiter'>(</span>value<span class='moth-code-delimiter'>)</span> <span class='moth-code-delimiter'>{</span><span class='moth-code-delimiter'>}</span>",
        "anonymous function must not colour value, got: {anonymous}"
    );

    let comment = highlight_code_html("function // note\nname", CodeLanguage::JavaScript);
    assert!(
        !comment.contains("<span class='moth-code-function'>name</span>"),
        "comment must interrupt the declaration lookahead, got: {comment}"
    );

    let newline = highlight_code_html("function\nname", CodeLanguage::JavaScript);
    assert!(
        !newline.contains("<span class='moth-code-function'>name</span>"),
        "newline must interrupt the declaration lookahead, got: {newline}"
    );

    let delimiter = highlight_code_html("function : name", CodeLanguage::JavaScript);
    assert!(
        !delimiter.contains("<span class='moth-code-function'>name</span>"),
        "delimiter must interrupt the declaration lookahead, got: {delimiter}"
    );

    let typescript = highlight_code_html("interface (User)", CodeLanguage::TypeScript);
    assert!(
        !typescript.contains("<span class='moth-code-contract'>User</span>"),
        "interface lookahead must not jump past a delimiter, got: {typescript}"
    );
}

#[test]
fn generic_profile_has_no_language_word_vocabulary() {
    let highlighted = highlight_code_html("function string true None", CodeLanguage::Generic);

    assert!(!highlighted.contains("moth-code-keyword"));
    assert!(!highlighted.contains("moth-code-type"));
    assert!(!highlighted.contains("moth-code-literal"));
    assert!(!highlighted.contains("moth-code-nominal"));
    assert!(highlighted.contains("function string true None"));
}

#[test]
fn html_markdown_and_toml_aliases_select_dedicated_profiles() {
    assert_eq!(CodeLanguage::from_alias("html"), Some(CodeLanguage::Html));
    assert_eq!(CodeLanguage::from_alias("md"), Some(CodeLanguage::Markdown));
    assert_eq!(
        CodeLanguage::from_alias("markdown"),
        Some(CodeLanguage::Markdown)
    );
    assert_eq!(CodeLanguage::from_alias("toml"), Some(CodeLanguage::Toml));
    assert_eq!(CodeLanguage::from_alias("json"), Some(CodeLanguage::Json));
    assert_eq!(CodeLanguage::from_alias("yaml"), Some(CodeLanguage::Yaml));
    assert_eq!(CodeLanguage::from_alias("yml"), Some(CodeLanguage::Yaml));
    assert_eq!(CodeLanguage::from_alias("css"), Some(CodeLanguage::Css));
    assert_eq!(CodeLanguage::from_alias("c"), Some(CodeLanguage::C));
    assert_eq!(CodeLanguage::from_alias("sql"), Some(CodeLanguage::Sql));
}

#[test]
fn json_profile_highlights_keys_literals_and_numbers() {
    let small = highlight_code_html(r#"{"name": "Priya"}"#, CodeLanguage::Json);
    assert_eq!(
        small,
        "<span class='moth-code-delimiter'>{</span><span class='moth-code-nominal'>&quot;name&quot;</span><span class='moth-code-operator'>:</span> <span class='moth-code-string'>&quot;Priya&quot;</span><span class='moth-code-delimiter'>}</span>",
        "a quoted key must be nominal and a value must stay a string, got: {small}"
    );

    let values = highlight_code_html(
        r#"{"age": 30, "active": true, "note": null}"#,
        CodeLanguage::Json,
    );
    assert!(values.contains("<span class='moth-code-number'>30</span>"));
    assert!(values.contains("<span class='moth-code-literal'>true</span>"));
    assert!(values.contains("<span class='moth-code-literal'>null</span>"));
}

#[test]
fn yaml_profile_highlights_keys_literals_markers_and_comments() {
    let yaml = highlight_code_html(
        r#"# config
name: Priya
- host: localhost
active: yes
---
..."#,
        CodeLanguage::Yaml,
    );
    assert!(yaml.contains("<span class='moth-code-comment'># config</span>"));
    assert!(
        yaml.contains(
            "<span class='moth-code-nominal'>name</span><span class='moth-code-operator'>:</span>"
        ),
        "a line-start mapping key must be nominal, got: {yaml}"
    );
    assert!(
        yaml.contains("<span class='moth-code-nominal'>host</span>"),
        "a list-item mapping key must be nominal, got: {yaml}"
    );
    assert!(yaml.contains("<span class='moth-code-literal'>yes</span>"));
    assert!(yaml.contains("<span class='moth-code-keyword'>---</span>"));
    assert!(yaml.contains("<span class='moth-code-keyword'>...</span>"));

    let quoted = highlight_code_html(r#""key": value"#, CodeLanguage::Yaml);
    assert!(
        quoted.contains("<span class='moth-code-nominal'>&quot;key&quot;</span>"),
        "a quoted line-start key must be nominal, got: {quoted}"
    );
}

#[test]
fn css_profile_highlights_comments_at_rules_and_properties() {
    let css = highlight_code_html(
        r#"/* note */
@media screen {
  color: red;
}"#,
        CodeLanguage::Css,
    );
    assert!(css.contains("<span class='moth-code-comment'>/* note */</span>"));
    assert!(css.contains("<span class='moth-code-keyword'>@media</span>"));
    assert!(
        css.contains(
            "<span class='moth-code-nominal'>color</span><span class='moth-code-operator'>:</span>"
        ),
        "a property inside a declaration block must be nominal, got: {css}"
    );
    assert!(
        !css.contains("<span class='moth-code-nominal'>screen</span>"),
        "selector words must stay plain, got: {css}"
    );
}

#[test]
fn c_profile_highlights_preprocessor_types_functions_and_comments() {
    let c = highlight_code_html(
        r#"#include <stdio.h>
int main(void) {
  // note
  printf("hi");
  return 0;
}"#,
        CodeLanguage::C,
    );
    assert!(c.contains("<span class='moth-code-keyword'>#include</span>"));
    assert!(c.contains("<span class='moth-code-type'>int</span>"));
    assert!(c.contains("<span class='moth-code-function'>main</span>"));
    assert!(c.contains("<span class='moth-code-comment'>// note</span>"));
    assert!(c.contains("<span class='moth-code-function'>printf</span>"));
    assert!(c.contains("<span class='moth-code-string'>&quot;hi&quot;</span>"));
    assert!(c.contains("<span class='moth-code-keyword'>return</span>"));

    let block = highlight_code_html("/* block */", CodeLanguage::C);
    assert_eq!(block, "<span class='moth-code-comment'>/* block */</span>");

    let nominal = highlight_code_html("Person", CodeLanguage::C);
    assert!(nominal.contains("<span class='moth-code-nominal'>Person</span>"));
}

#[test]
fn sql_profile_highlights_keywords_functions_literals_and_comments() {
    let sql = highlight_code_html(
        "SELECT name, COUNT(*) FROM users WHERE active = true; -- note",
        CodeLanguage::Sql,
    );
    assert!(sql.contains("<span class='moth-code-keyword'>SELECT</span>"));
    assert!(sql.contains("<span class='moth-code-function'>COUNT</span>"));
    assert!(sql.contains("<span class='moth-code-keyword'>FROM</span>"));
    assert!(sql.contains("<span class='moth-code-keyword'>WHERE</span>"));
    assert!(sql.contains("<span class='moth-code-literal'>true</span>"));
    assert!(sql.contains("<span class='moth-code-comment'>-- note</span>"));

    let lowercase = highlight_code_html("select name from users", CodeLanguage::Sql);
    assert!(lowercase.contains("<span class='moth-code-keyword'>select</span>"));
    assert!(lowercase.contains("<span class='moth-code-keyword'>from</span>"));
}

#[test]
fn html_profile_highlights_comments_declarations_and_tags() {
    let comment = highlight_code_html("<!-- note -->", CodeLanguage::Html);
    assert_eq!(
        comment, "<span class='moth-code-comment'>&lt;!-- note --&gt;</span>",
        "an HTML comment must be one comment span, got: {comment}"
    );

    let declaration = highlight_code_html("<!DOCTYPE html>", CodeLanguage::Html);
    assert_eq!(
        declaration, "<span class='moth-code-keyword'>&lt;!DOCTYPE html&gt;</span>",
        "a declaration must be one keyword span, got: {declaration}"
    );

    let tag = highlight_code_html("<div class=\"card\">text</div>", CodeLanguage::Html);
    assert_eq!(
        tag,
        "<span class='moth-code-delimiter'>&lt;</span><span class='moth-code-type'>div</span> <span class='moth-code-nominal'>class</span><span class='moth-code-operator'>=</span><span class='moth-code-string'>&quot;card&quot;</span><span class='moth-code-delimiter'>&gt;</span>text<span class='moth-code-delimiter'>&lt;/</span><span class='moth-code-type'>div</span><span class='moth-code-delimiter'>&gt;</span>",
        "tag parts must use the shared palette, got: {tag}"
    );

    let prose = highlight_code_html("Hello!", CodeLanguage::Html);
    assert_eq!(
        prose, "Hello!",
        "HTML prose must keep `!` plain, got: {prose}"
    );
}

#[test]
fn markdown_profile_highlights_headings_and_inline_code() {
    let heading = highlight_code_html("# Title\nbody", CodeLanguage::Markdown);
    assert_eq!(
        heading, "<span class='moth-code-keyword'>#</span> Title\nbody",
        "an ATX heading marker must be a keyword, got: {heading}"
    );

    let code = highlight_code_html("use `code` now", CodeLanguage::Markdown);
    assert_eq!(
        code, "use <span class='moth-code-string'>`code`</span> now",
        "an inline code span must be a string, got: {code}"
    );

    let unclosed = highlight_code_html("`oops", CodeLanguage::Markdown);
    assert_eq!(
        unclosed, "<span class='moth-code-operator'>`</span>oops",
        "an unclosed backtick must stay an operator, got: {unclosed}"
    );
}

#[test]
fn toml_profile_highlights_comments_tables_keys_and_values() {
    let comment = highlight_code_html("# config", CodeLanguage::Toml);
    assert_eq!(comment, "<span class='moth-code-comment'># config</span>");

    let table = highlight_code_html("[server]", CodeLanguage::Toml);
    assert_eq!(
        table, "<span class='moth-code-keyword'>[server]</span>",
        "a table header must be one keyword span, got: {table}"
    );

    let array_table = highlight_code_html("[[items]]", CodeLanguage::Toml);
    assert_eq!(
        array_table, "<span class='moth-code-keyword'>[[items]]</span>",
        "an array-of-tables header must be one keyword span, got: {array_table}"
    );

    let pair = highlight_code_html("host = \"localhost\"", CodeLanguage::Toml);
    assert_eq!(
        pair,
        "<span class='moth-code-nominal'>host</span> <span class='moth-code-operator'>=</span> <span class='moth-code-string'>&quot;localhost&quot;</span>",
        "a key, operator and string value must use the shared palette, got: {pair}"
    );

    let boolean = highlight_code_html("enabled = true", CodeLanguage::Toml);
    assert_eq!(
        boolean,
        "<span class='moth-code-nominal'>enabled</span> <span class='moth-code-operator'>=</span> <span class='moth-code-literal'>true</span>",
        "a boolean value must be a literal, got: {boolean}"
    );

    let dotted = highlight_code_html("a.b = 1", CodeLanguage::Toml);
    assert_eq!(
        dotted,
        "<span class='moth-code-nominal'>a</span>.<span class='moth-code-nominal'>b</span> <span class='moth-code-operator'>=</span> <span class='moth-code-number'>1</span>",
        "dotted key segments must be nominal, got: {dotted}"
    );
}

#[test]
fn prose_like_profiles_keep_ordinary_words_plain() {
    for (language, source) in [
        (CodeLanguage::Html, "Hello world"),
        (CodeLanguage::Markdown, "Hello world"),
        (CodeLanguage::Toml, "Hello world"),
        (CodeLanguage::Json, "Hello world"),
        (CodeLanguage::Yaml, "Hello world"),
        (CodeLanguage::Css, "Hello world"),
        (CodeLanguage::Sql, "Hello world"),
    ] {
        let highlighted = highlight_code_html(source, language);
        assert!(
            !highlighted.contains("moth-code-nominal"),
            "{language:?} must not colour prose words nominal, got: {highlighted}"
        );
    }
}

#[test]
fn new_language_profiles_preserve_every_source_byte_exactly_once() {
    let cases = [
        (
            CodeLanguage::Html,
            r#"<div class="card">text</div>
<!-- note -->"#,
        ),
        (
            CodeLanguage::Markdown,
            r#"# Title
use `code` now
`oops"#,
        ),
        (
            CodeLanguage::Toml,
            r#"[server]
host = "localhost"
enabled = true
# note"#,
        ),
        (CodeLanguage::Json, r#"{"name": "Priya", "age": 30}"#),
        (
            CodeLanguage::Yaml,
            r#"name: Priya
active: yes
---"#,
        ),
        (
            CodeLanguage::Css,
            r#"/* note */
.card { color: red; }"#,
        ),
        (
            CodeLanguage::C,
            r#"#include <stdio.h>
int main(void) { return 0; }"#,
        ),
        (CodeLanguage::Sql, "SELECT name FROM users; -- note"),
    ];

    for (language, source) in cases {
        let highlighted = highlight_code_html(source, language);
        let stripped = strip_role_spans(&highlighted);
        let expected = escape_source_for_comparison(source);
        assert_eq!(
            stripped, expected,
            "span-free output must equal escaped input for {language:?} {source:?}\ngot: {stripped}\nhighlighted: {highlighted}"
        );
    }
}

#[test]
fn moth_pipe_groups_keep_captures_and_parameters_plain() {
    let declaration = highlight_code_html("render |value|", CodeLanguage::Moth);
    assert_eq!(
        declaration,
        "<span class='moth-code-function'>render</span> <span class='moth-code-delimiter'>|</span>value<span class='moth-code-delimiter'>|</span>",
        "only the declaration name before a pipe group is a function, got: {declaration}"
    );

    let capture = highlight_code_html("if option is |value|", CodeLanguage::Moth);
    assert_eq!(
        capture,
        "<span class='moth-code-keyword'>if</span> option <span class='moth-code-operator'>is</span> <span class='moth-code-delimiter'>|</span>value<span class='moth-code-delimiter'>|</span>",
        "captured value must stay plain inside pipes, got: {capture}"
    );

    let parameters = highlight_code_html("render |title, count| -> String:", CodeLanguage::Moth);
    assert!(
        parameters.contains("<span class='moth-code-function'>render</span>"),
        "declaration name must stay a function, got: {parameters}"
    );
    assert!(
        !parameters.contains("<span class='moth-code-function'>title</span>")
            && !parameters.contains("<span class='moth-code-function'>count</span>"),
        "untyped parameters inside pipes must stay plain, got: {parameters}"
    );
}

#[test]
fn moth_generic_bounds_are_contracts_only_in_generic_declarations() {
    let comparison = highlight_code_html("value is MAX_SIZE", CodeLanguage::Moth);
    assert!(
        !comparison.contains("<span class='moth-code-contract'>MAX_SIZE</span>"),
        "ordinary is comparison must not colour MAX_SIZE as a contract, got: {comparison}"
    );

    let generic = highlight_code_html(
        "render type Item is DISPLAY_TEXT |value A|",
        CodeLanguage::Moth,
    );
    assert!(
        generic.contains("<span class='moth-code-contract'>DISPLAY_TEXT</span>"),
        "generic bound trait must be a contract, got: {generic}"
    );
    assert!(
        !generic.contains("<span class='moth-code-contract'>A</span>"),
        "generic parameter after the bound list must not be a contract, got: {generic}"
    );
}

#[test]
fn moth_conformance_and_generic_commas_classify_differently() {
    let conformance = highlight_code_html("Label must FIRST, SECOND", CodeLanguage::Moth);
    assert!(
        conformance.contains("<span class='moth-code-contract'>FIRST</span>"),
        "FIRST must be a conformance contract, got: {conformance}"
    );
    assert!(
        conformance.contains("<span class='moth-code-contract'>SECOND</span>"),
        "SECOND must continue the conformance list, got: {conformance}"
    );

    let generic = highlight_code_html("type A is FIRST, B is SECOND", CodeLanguage::Moth);
    assert!(
        generic.contains("<span class='moth-code-contract'>FIRST</span>"),
        "FIRST must be a generic bound contract, got: {generic}"
    );
    assert!(
        generic.contains("<span class='moth-code-nominal'>B</span>"),
        "B must return to a nominal generic parameter after the comma, got: {generic}"
    );
    assert!(
        generic.contains("<span class='moth-code-contract'>SECOND</span>"),
        "SECOND must be a contract after the second is, got: {generic}"
    );
}

#[test]
fn moth_loop_sources_are_not_function_declarations() {
    let plain = highlight_code_html("loop items |item|:", CodeLanguage::Moth);
    assert_eq!(
        plain,
        "<span class='moth-code-keyword'>loop</span> items <span class='moth-code-delimiter'>|</span>item<span class='moth-code-delimiter'>|</span><span class='moth-code-delimiter'>:</span>",
        "loop source must stay plain, got: {plain}"
    );

    let projection = highlight_code_html("loop collection.items |item|:", CodeLanguage::Moth);
    assert!(
        !projection.contains("<span class='moth-code-function'>collection</span>")
            && !projection.contains("<span class='moth-code-function'>items</span>"),
        "loop projections must stay plain, got: {projection}"
    );

    let call = highlight_code_html("loop get_items() |item|:", CodeLanguage::Moth);
    assert!(
        call.contains("<span class='moth-code-function'>get_items</span>"),
        "the loop source call must keep its function role, got: {call}"
    );
    assert!(
        !call.contains("<span class='moth-code-function'>item</span>"),
        "the loop binding must stay plain, got: {call}"
    );

    // The header state must end at `:` so a later declaration is unaffected.
    let following = highlight_code_html("loop items |item|:\nrender |value|", CodeLanguage::Moth);
    assert!(
        following.contains("<span class='moth-code-function'>render</span>"),
        "loop header state must reset at the colon, got: {following}"
    );
}

#[test]
fn moth_generic_function_owners_are_functions() {
    let free = highlight_code_html("identity type A |value A| -> A:", CodeLanguage::Moth);
    assert!(
        free.contains("<span class='moth-code-function'>identity</span>"),
        "generic free-function owner must be a function, got: {free}"
    );

    let method = highlight_code_html(
        "render type Item is DISPLAY_TEXT |item Item| -> String:",
        CodeLanguage::Moth,
    );
    assert!(
        method.contains("<span class='moth-code-function'>render</span>"),
        "generic declaration owner must be a function, got: {method}"
    );
    assert!(
        method.contains("<span class='moth-code-contract'>DISPLAY_TEXT</span>"),
        "generic bound must stay a contract, got: {method}"
    );
}

#[test]
fn moth_contract_names_use_compiler_uppercase_policy() {
    let single = highlight_code_html("A must:", CodeLanguage::Moth);
    assert!(
        single.contains("<span class='moth-code-contract'>A</span>"),
        "single-letter trait name must be a contract, got: {single}"
    );

    let conformance = highlight_code_html("Label must A", CodeLanguage::Moth);
    assert!(
        conformance.contains("<span class='moth-code-contract'>A</span>"),
        "single-letter conformance name must be a contract, got: {conformance}"
    );

    for name in ["TRAIT2", "HTTP_2"] {
        let highlighted = highlight_code_html(&format!("Label must {name}"), CodeLanguage::Moth);
        assert!(
            highlighted.contains(&format!("<span class='moth-code-contract'>{name}</span>")),
            "{name} must be a contract under the compiler naming policy, got: {highlighted}"
        );
    }
}

#[test]
fn moth_conformance_lists_survive_comma_continued_newlines() {
    let continued = highlight_code_html("Label must FIRST,\n    SECOND", CodeLanguage::Moth);
    assert!(
        continued.contains("<span class='moth-code-contract'>SECOND</span>"),
        "comma-continued conformance must survive the newline, got: {continued}"
    );

    let reset = highlight_code_html("Label must FIRST\n    SECOND", CodeLanguage::Moth);
    assert!(
        !reset.contains("<span class='moth-code-contract'>SECOND</span>"),
        "newline without a preceding comma must reset conformance state, got: {reset}"
    );
}

#[test]
fn moth_path_boundaries_are_unicode_aware() {
    assert_eq!(
        highlight_code_html("π@core/io", CodeLanguage::Moth),
        "π<span class='moth-code-operator'>@</span>core<span class='moth-code-operator'>/</span>io",
        "a path must not start after a Unicode identifier continuation"
    );
    assert_eq!(
        highlight_code_html("name@core/io", CodeLanguage::Moth),
        "name<span class='moth-code-operator'>@</span>core<span class='moth-code-operator'>/</span>io",
        "a path must not start after an ASCII identifier continuation"
    );
    assert_eq!(
        highlight_code_html("@@core/io", CodeLanguage::Moth),
        "<span class='moth-code-operator'>@</span><span class='moth-code-operator'>@</span>core<span class='moth-code-operator'>/</span>io",
        "a doubled @ must not present a valid-looking path"
    );
    assert_eq!(
        highlight_code_html("(@core/io)", CodeLanguage::Moth),
        "<span class='moth-code-delimiter'>(</span><span class='moth-code-string'>@core/io</span><span class='moth-code-delimiter'>)</span>",
        "a path after a delimiter must highlight"
    );
    assert_eq!(
        highlight_code_html("import @core/io", CodeLanguage::Moth),
        "<span class='moth-code-keyword'>import</span> <span class='moth-code-string'>@core/io</span>",
        "a path after whitespace must highlight"
    );
}

/// Removes highlighter role spans, returning the escaped text written by the scanner.
///
/// WHY: source preservation is measured by comparing the span-free output with an
///      independent escape of the input.
fn strip_role_spans(highlighted: &str) -> String {
    let mut stripped = String::with_capacity(highlighted.len());
    let mut rest = highlighted;

    while let Some(open_start) = rest.find("<span class='moth-code-") {
        stripped.push_str(&rest[..open_start]);
        let open_end = rest[open_start..]
            .find("'>")
            .expect("role span always closes its opening tag")
            + open_start
            + 2;
        let after_open = &rest[open_end..];
        let close_start = after_open
            .find("</span>")
            .expect("role span always has a closing tag");
        stripped.push_str(&after_open[..close_start]);
        rest = &after_open[close_start + "</span>".len()..];
    }

    stripped.push_str(rest);
    stripped
}

/// Independent oracle for the five HTML escapes used by the highlighter.
fn escape_source_for_comparison(source: &str) -> String {
    let mut escaped = String::with_capacity(source.len());

    for ch in source.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }

    escaped
}
