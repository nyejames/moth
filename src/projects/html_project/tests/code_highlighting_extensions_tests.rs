//! Presentation regressions for final Moth syntax and foreign-language extensions.
//!
//! Incomplete and deferred snippets exercise highlighting, not compiler acceptance.

use crate::projects::html_project::styles::code::{CodeLanguage, highlight_code_html};

#[test]
fn final_moth_placement_and_decimal_names_have_distinct_roles() {
    assert_eq!(
        highlight_code_html("into", CodeLanguage::Moth),
        "<span class='moth-code-keyword'>into</span>"
    );
    for name in
        std::iter::once("Dec".to_owned()).chain((0..=256).map(|scale| format!("Dec{scale}")))
    {
        assert_eq!(
            highlight_code_html(&name, CodeLanguage::Moth),
            format!("<span class='moth-code-type'>{name}</span>")
        );
    }
    for name in [
        "Dec01",
        "Dec257",
        "Decimal",
        "dec2",
        "Dec２",
        "Dec999999999999",
    ] {
        assert!(!highlight_code_html(name, CodeLanguage::Moth).contains("moth-code-type"));
    }
    assert_eq!(
        highlight_code_html("block group region into_value", CodeLanguage::Moth),
        "block group region into_value"
    );
}

#[test]
fn moth_error_bangs_are_separate_from_keywords_types_and_delimiters() {
    let highlighted =
        highlight_code_html("return! cast! Error! read()! !: -> !", CodeLanguage::Moth);
    assert_eq!(
        highlighted
            .matches("<span class='moth-code-error'>!</span>")
            .count(),
        6
    );
    for keyword in ["return", "cast"] {
        assert!(highlighted.contains(&format!(
            "<span class='moth-code-keyword'>{keyword}</span><span class='moth-code-error'>!</span>"
        )));
    }
    assert!(highlighted.contains(
        "<span class='moth-code-type'>Error</span><span class='moth-code-error'>!</span>"
    ));
    assert!(highlighted.contains(
        "<span class='moth-code-error'>!</span><span class='moth-code-delimiter'>:</span>"
    ));
}

#[test]
fn moth_future_word_roles_do_not_override_dependency_names() {
    let highlighted = highlight_code_html(
        "@core/tool into as placement, Dec2 as Money",
        CodeLanguage::Moth,
    );
    for name in ["into", "placement", "Dec2", "Money"] {
        assert!(highlighted.contains(&format!("<span class='moth-code-nominal'>{name}</span>")));
    }
}

#[test]
fn strings_comments_and_paths_shield_moth_error_markers() {
    let highlighted = highlight_code_html(
        "\"return! into Dec2 <&>\" -- cast! into Dec2\n@core/into",
        CodeLanguage::Moth,
    );
    assert!(highlighted.contains(
        "<span class='moth-code-string'>&quot;return! into Dec2 &lt;&amp;&gt;&quot;</span>"
    ));
    assert!(highlighted.contains("<span class='moth-code-comment'>-- cast! into Dec2</span>"));
    assert!(highlighted.contains("<span class='moth-code-string'>@core/into</span>"));
    for role in ["error", "keyword", "type"] {
        assert!(!highlighted.contains(&format!("moth-code-{role}")));
    }
}

#[test]
fn final_moth_directives_wiring_and_numeric_families_keep_their_roles() {
    let highlighted = highlight_code_html(
        "$fast_math $safe_math $infallible of Uint U64 F16 Byte Wire Route Channel",
        CodeLanguage::Moth,
    );
    for name in ["$fast_math", "$safe_math", "$infallible"] {
        assert!(highlighted.contains(&format!("<span class='moth-code-directive'>{name}</span>")));
    }
    assert!(highlighted.contains("<span class='moth-code-keyword'>of</span>"));
    for name in ["Uint", "U64", "F16", "Byte"] {
        assert!(highlighted.contains(&format!("<span class='moth-code-type'>{name}</span>")));
    }
    for name in ["Wire", "Route", "Channel"] {
        assert!(highlighted.contains(&format!("<span class='moth-code-nominal'>{name}</span>")));
    }
}

#[test]
fn foreign_profiles_share_error_keywords_without_reclassifying_negation() {
    for (language, words) in [
        (CodeLanguage::JavaScript, "throw catch"),
        (CodeLanguage::TypeScript, "throw catch"),
        (CodeLanguage::Python, "raise except"),
    ] {
        for word in words.split_whitespace() {
            assert_eq!(
                highlight_code_html(word, language),
                format!("<span class='moth-code-error'>{word}</span>")
            );
        }
    }
    for language in [
        CodeLanguage::JavaScript,
        CodeLanguage::TypeScript,
        CodeLanguage::Rust,
        CodeLanguage::C,
        CodeLanguage::Shell,
    ] {
        let highlighted = highlight_code_html("!ready", language);
        assert!(!highlighted.contains("moth-code-error"));
        assert!(highlighted.contains("<span class='moth-code-operator'>!</span>"));
    }
    assert!(
        !highlight_code_html("println!(\"hi\")", CodeLanguage::Rust).contains("moth-code-error")
    );
}

#[test]
fn foreign_profiles_cover_missing_control_and_declaration_words() {
    for (language, words) in [
        (
            CodeLanguage::JavaScript,
            "async await class extends import export switch try finally",
        ),
        (
            CodeLanguage::TypeScript,
            "async await class implements readonly keyof infer satisfies",
        ),
        (
            CodeLanguage::Python,
            "async await with yield lambda pass try finally",
        ),
        (CodeLanguage::Rust, "loop as"),
        (CodeLanguage::Shell, "case esac until select"),
    ] {
        for word in words.split_whitespace() {
            assert_eq!(
                highlight_code_html(word, language),
                format!("<span class='moth-code-keyword'>{word}</span>"),
                "{language:?}: {word}"
            );
        }
    }
}

#[test]
fn class_name_lookahead_skips_reserved_words_and_typescript_words_stay_isolated() {
    assert_eq!(
        highlight_code_html("class point extends Base", CodeLanguage::JavaScript),
        "<span class='moth-code-keyword'>class</span> <span class='moth-code-nominal'>point</span> \
         <span class='moth-code-keyword'>extends</span> <span class='moth-code-nominal'>Base</span>"
    );
    assert_eq!(
        highlight_code_html("class extends base", CodeLanguage::JavaScript),
        "<span class='moth-code-keyword'>class</span> <span class='moth-code-keyword'>extends</span> base"
    );
    assert_eq!(
        highlight_code_html(
            "type interface readonly number is",
            CodeLanguage::JavaScript
        ),
        "type interface readonly number is"
    );
}

#[test]
fn block_comments_shield_keywords_and_html_in_c_style_profiles() {
    for language in [
        CodeLanguage::JavaScript,
        CodeLanguage::TypeScript,
        CodeLanguage::Rust,
        CodeLanguage::C,
        CodeLanguage::Css,
        CodeLanguage::Sql,
    ] {
        assert_eq!(
            highlight_code_html("/* throw ! <&>\r\nreturn */", language),
            "<span class='moth-code-comment'>/* throw ! &lt;&amp;&gt;\r\nreturn */</span>",
            "{language:?}"
        );
        assert_eq!(
            highlight_code_html("/* unfinished ! π", language),
            "<span class='moth-code-comment'>/* unfinished ! π</span>",
            "{language:?}"
        );
    }
}

#[test]
fn rust_comments_nest_while_javascript_closes_at_the_first_terminator() {
    let source = "/* outer /* inner */ tail */";
    assert_eq!(
        highlight_code_html(source, CodeLanguage::Rust),
        "<span class='moth-code-comment'>/* outer /* inner */ tail */</span>"
    );
    let javascript = highlight_code_html(source, CodeLanguage::JavaScript);
    assert!(
        javascript.starts_with("<span class='moth-code-comment'>/* outer /* inner */</span> tail ")
    );
    assert_eq!(
        highlight_code_html("/* outer /* inner */", CodeLanguage::Rust),
        "<span class='moth-code-comment'>/* outer /* inner */</span>"
    );
}

#[test]
fn moth_template_bodies_are_plain_text_around_highlighted_interpolations() {
    assert_eq!(
        highlight_code_html(
            "[: Hello, [name]! Don't -- stop \"now\"]",
            CodeLanguage::Moth
        ),
        "<span class='moth-code-delimiter'>[</span><span class='moth-code-delimiter'>:</span> \
         Hello, <span class='moth-code-delimiter'>[</span>name<span class='moth-code-delimiter'>]</span>\
         ! Don&#39;t -- stop &quot;now&quot;<span class='moth-code-delimiter'>]</span>"
    );

    // Heads keep code roles, including nested templates inside head arguments,
    // while body markup and quotes stay plain.
    let highlighted = highlight_code_html(
        "[$children([:<li>[$slot]</li>]): <ul class=\"x\">[$slot]</ul>]",
        CodeLanguage::Moth,
    );
    assert_eq!(
        highlighted
            .matches("<span class='moth-code-directive'>$slot</span>")
            .count(),
        2
    );
    assert!(highlighted.contains("<span class='moth-code-directive'>$children</span>"));
    assert!(highlighted.contains(
        "<span class='moth-code-delimiter'>:</span> &lt;ul class=&quot;x&quot;&gt;<span class='moth-code-delimiter'>[</span>"
    ));
    assert!(!highlighted.contains("moth-code-operator"));
    assert!(!highlighted.contains("moth-code-string"));
}

#[test]
fn moth_template_heads_keep_control_flow_strings_and_code_resumes_after_close() {
    let highlighted = highlight_code_html(
        "[loop items |item, index|:\n    [index]: [item] if ready!\n]\n[\"[literal]\"]\nreturn! value",
        CodeLanguage::Moth,
    );
    assert!(highlighted.contains("<span class='moth-code-keyword'>loop</span>"));
    assert!(highlighted.contains(
        "<span class='moth-code-delimiter'>]</span>: <span class='moth-code-delimiter'>[</span>"
    ));
    assert!(highlighted.contains("</span> if ready!\n<span class='moth-code-delimiter'>]</span>"));
    assert!(highlighted.contains("<span class='moth-code-string'>&quot;[literal]&quot;</span>"));
    assert!(highlighted.ends_with(
        "<span class='moth-code-keyword'>return</span><span class='moth-code-error'>!</span> value"
    ));
    assert_eq!(highlighted.matches("moth-code-error").count(), 1);
}

#[test]
fn unfinished_moth_template_body_stays_plain_to_the_end_of_the_snippet() {
    assert_eq!(
        highlight_code_html("[: unfinished ! π <&>", CodeLanguage::Moth),
        "<span class='moth-code-delimiter'>[</span><span class='moth-code-delimiter'>:</span> \
         unfinished ! π &lt;&amp;&gt;"
    );
}

#[test]
fn rust_lifetimes_stay_plain_while_character_literals_are_strings() {
    let highlighted = highlight_code_html(
        "fn first<'a>(x: &'a str) -> &'static str { let c = 'x'; let n = '\\n'; let p = 'π'; }",
        CodeLanguage::Rust,
    );
    for literal in ["'x'", "'\\n'", "'π'"] {
        assert!(
            highlighted.contains(&format!(
                "<span class='moth-code-string'>{}</span>",
                literal.replace('\'', "&#39;")
            )),
            "{literal} in: {highlighted}"
        );
    }
    assert_eq!(highlighted.matches("moth-code-string").count(), 3);
    assert!(highlighted.contains("&#39;static <span class='moth-code-type'>str</span>"));
    assert!(!highlighted.contains("<span class='moth-code-keyword'>static</span>"));
}

#[test]
fn javascript_template_literals_are_single_string_runs() {
    for language in [CodeLanguage::JavaScript, CodeLanguage::TypeScript] {
        assert_eq!(
            highlight_code_html("`Hi ${name}! <&> \\` done`", language),
            "<span class='moth-code-string'>`Hi ${name}! &lt;&amp;&gt; \\` done`</span>",
            "{language:?}"
        );
    }
    assert!(highlight_code_html("`ls`", CodeLanguage::Shell).contains("moth-code-operator"));
}
