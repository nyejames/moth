//! Scaffold templates for `moth new html`.
//!
//! WHAT: Owns the generated content for `config.moth`, `src/@page.moth`, and `.gitignore`.
//! WHY: Centralises template strings so they are not scattered through write logic.

/// Escape a string for use in a Moth config string literal.
///
/// Minimum escaping: backslash and double-quote.
fn escape_moth_string_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Generate the content for `config.moth`.
pub fn config_template(project_name: &str) -> String {
    let escaped = escape_moth_string_literal(project_name);
    format!(
        r#"project #= |
    name = "{escaped}",
    entry_root = "src",
    version = "0.1.0",
    author = "",
    license = "MIT",
|
html #= |
    page_url_style = "trailing_slash",
    redirect_index_html = true,
    html_lang = "en",
|
"#
    )
}

/// Return the exact starter content for `src/@page.moth`.
pub fn page_template() -> &'static str {
    r#"page_title #= "Welcome"
page_head #= [$html:
    <style>
        [$css:
            body {
                background-color: light-dark(hsl(125, 67%, 97%), hsl(203, 68%, 8%));
                padding: var(--moth-spacing);
            }
        ]
    </style>
]

[$md:
    # Welcome to Moth

    Here's the @https://nyejames.github.io/moth/docs/ (documentation).

    Use **moth dev** to start the development server and see your changes to this page in real time!
]
"#
}

/// Return the default `.gitignore` content for a new HTML project.
pub fn gitignore_template() -> &'static str {
    "# Moth development output\n/dev\n\n# OS/editor noise\n.DS_Store\n.vscode/\n.idea/\n"
}

/// Return the block appended to an existing `.gitignore` when it is missing.
pub fn gitignore_append_block() -> &'static str {
    "\n# Moth\n/dev\n"
}
