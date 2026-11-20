//! HTML project scaffolding orchestration.
//!
//! WHAT: Coordinates target resolution, user prompting, and file creation for `moth new html`.
//! WHY: Keeps CLI dispatch thin and makes the scaffold flow testable without stdin.

pub mod options;
pub mod prompt;

pub(crate) mod files;
pub(crate) mod start_page_scaffolding;
pub(crate) mod target;

pub use options::NewHtmlProjectOptions;
pub use prompt::{Prompt, TerminalPrompt};

use std::path::{Path, PathBuf};

/// Report of what the scaffold operation created, updated, skipped, or replaced.
#[derive(Debug)]
pub struct CreateProjectReport {
    pub project_path: PathBuf,
    pub project_name: String,
    pub created: Vec<PathBuf>,
    pub updated: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub replaced: Vec<PathBuf>,
}

/// Create a new HTML project using the terminal for prompts.
pub fn create_html_project_template(options: NewHtmlProjectOptions) -> Result<(), String> {
    let mut prompt = TerminalPrompt::new();
    let report = create_html_project_template_with_prompt(options, &mut prompt)?;
    println!(
        "{}",
        render_summary(&report, std::env::current_dir().ok().as_deref())
    );
    Ok(())
}

/// Create a new HTML project with an injected prompt implementation.
pub fn create_html_project_template_with_prompt(
    options: NewHtmlProjectOptions,
    prompt: &mut impl Prompt,
) -> Result<CreateProjectReport, String> {
    let current_dir =
        std::env::current_dir().map_err(|e| format!("Failed to resolve current directory: {e}"))?;

    let resolved = target::resolve_project_target(options.raw_path, &current_dir, prompt)?;

    files::run_preflight_checks(&resolved, options.force, prompt)?;

    files::write_scaffold(&resolved, options.force, prompt)
}

/// Render the CLI summary for a successful scaffold.
///
/// WHAT: builds the user-facing text that explains exactly what was created,
/// updated, skipped, or replaced, plus the next steps.
/// WHY: `new` is often a user's first interaction with `moth`; the output must
/// be explicit, calm, and actionable.
pub fn render_summary(report: &CreateProjectReport, current_dir: Option<&Path>) -> String {
    let mut lines = Vec::new();

    lines.push(String::from("Created Moth HTML project:"));
    push_summary_line(
        &mut lines,
        "  Project path: ",
        &report.project_path.to_string_lossy(),
    );
    push_summary_line(&mut lines, "  Project name: ", &report.project_name);

    if !report.created.is_empty() {
        lines.push(String::new());
        lines.push(String::from("Created:"));
        for path in &report.created {
            push_summary_line(&mut lines, "  ", &path.to_string_lossy());
        }
    }

    if !report.updated.is_empty() {
        lines.push(String::new());
        lines.push(String::from("Updated:"));
        for path in &report.updated {
            push_summary_line(&mut lines, "  ", &path.to_string_lossy());
        }
    }

    if !report.replaced.is_empty() {
        lines.push(String::new());
        lines.push(String::from("Replaced:"));
        for path in &report.replaced {
            push_summary_line(&mut lines, "  ", &path.to_string_lossy());
        }
    }

    if !report.skipped.is_empty() {
        lines.push(String::new());
        lines.push(String::from("Skipped:"));
        for path in &report.skipped {
            push_summary_line(&mut lines, "  ", &path.to_string_lossy());
        }
    }

    lines.push(String::new());
    lines.push(String::from("Next:"));

    let is_current_dir = match current_dir {
        Some(cd) => match (
            std::fs::canonicalize(cd),
            std::fs::canonicalize(&report.project_path),
        ) {
            (Ok(cd_canon), Ok(proj_canon)) => cd_canon == proj_canon,
            _ => false,
        },
        None => false,
    };

    if !is_current_dir {
        if let Some(command) = change_directory_command(&report.project_path) {
            lines.push(command);
        } else {
            lines.push(String::from(
                "  Open the project directory in your terminal before running these commands:",
            ));
        }
    }
    lines.push(String::from("  moth check ."));
    lines.push(String::from("  moth dev ."));

    lines.join("\n")
}

fn push_summary_line(lines: &mut Vec<String>, prefix: &str, text: &str) {
    let mut line = String::with_capacity(prefix.len() + text.len());
    line.push_str(prefix);
    for character in text.chars() {
        if is_unsafe_terminal_character(character) {
            line.extend(character.escape_default());
        } else {
            line.push(character);
        }
    }
    lines.push(line);
}

fn is_unsafe_terminal_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}

/// Emit a literal POSIX shell or PowerShell command, not a display spelling.
fn change_directory_command(path: &Path) -> Option<String> {
    #[cfg(windows)]
    let absolute;
    #[cfg(windows)]
    let path = if path.is_absolute() {
        path
    } else {
        // Resolve drive-relative targets with the same process context used to create them.
        absolute = std::path::absolute(path).ok()?;
        &absolute
    };
    let path = path.to_str()?;
    if path.chars().any(is_unsafe_terminal_character) {
        return None;
    }

    #[cfg(windows)]
    {
        // Qualify UNC paths even when the caller has a non-filesystem provider selected.
        let prefix = "  Set-Location -LiteralPath 'FileSystem::";
        let mut command = String::with_capacity(prefix.len() + path.len() + 1);
        command.push_str(prefix);
        for character in path.chars() {
            // PowerShell recognises these smart quotes as single-quote delimiters too.
            if matches!(
                character,
                '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}'
            ) {
                command.push(character);
            }
            command.push(character);
        }
        command.push('\'');
        Some(command)
    }
    #[cfg(not(windows))]
    {
        let mut command = String::with_capacity("  cd ''".len() + path.len());
        command.push_str("  cd '");
        for character in path.chars() {
            if character == '\'' {
                command.push_str("'\\''");
            } else {
                command.push(character);
            }
        }
        command.push('\'');
        Some(command)
    }
}

#[cfg(test)]
#[path = "tests/prompt_tests.rs"]
pub mod prompt_tests;

#[cfg(test)]
#[path = "tests/target_tests.rs"]
pub mod target_tests;

#[cfg(test)]
#[path = "tests/summary_tests.rs"]
pub mod summary_tests;

#[cfg(test)]
#[path = "tests/scaffold_tests.rs"]
pub mod scaffold_tests;
