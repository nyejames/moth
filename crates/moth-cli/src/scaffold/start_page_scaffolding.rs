//! Scaffold templates for `moth new html`.
//!
//! WHAT: Owns the generated content for `config.moth`, the root-level `@entry.moth`, and
//!       `.gitignore`.
//! WHY: Centralises template strings so they are not scattered through write logic.

/// Generate the content for `config.moth`.
///
/// The scaffold always writes an explicit valid project name so the created directory has a
/// stable identity even though directory compilation does not require a config file.
pub fn config_template(project_name: &str) -> Result<String, String> {
    if !moth_lexical::identifier::is_identifier(project_name) {
        return Err(format!(
            "Invalid project name `{project_name}`. Use a Moth project identifier: \
             letters, digits and underscores, not starting with a digit."
        ));
    }
    Ok(format!(
        r#"project #= (
    name = "{project_name}",
)
html #= ()
"#
    ))
}

/// Return the exact starter content for the root-level `@entry.moth`.
pub fn page_template() -> &'static str {
    r#"[$in title:Welcome]
starter_head #= [$html:
    <style>
        [$css:
            body {
                background-color: light-dark(hsl(125, 67%, 97%), hsl(203, 68%, 8%));
                padding: var(--moth-spacing);
            }
        ]
    </style>
]
[$in head:[starter_head]]

[$md:
    # Welcome to Moth

    Here's the @https://nyejames.github.io/moth/docs/ (documentation).

    Use **moth dev** to start the development server and see your changes to this page in real time!
]
"#
}

/// Return the default `.gitignore` content for a new HTML project.
pub fn gitignore_template() -> &'static str {
    "# Moth generated output\n/_dev\n/_release\n\n# OS/editor noise\n.DS_Store\n.vscode/\n.idea/\n"
}
