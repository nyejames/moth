//! HTML project scaffolding orchestration.
//!
//! WHAT: Coordinates target resolution, user prompting, and file creation for `moth new html`.
//! WHY: Keeps CLI dispatch thin and makes the scaffold flow testable without stdin.

pub mod options;
pub mod prompt;

pub(crate) mod scaffold;
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

    scaffold::run_preflight_checks(&resolved, options.force, prompt)?;

    scaffold::write_scaffold(&resolved, options.force, prompt)
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
    lines.push(format!("  Project path: {}", report.project_path.display()));
    lines.push(format!("  Project name: {}", report.project_name));

    if !report.created.is_empty() {
        lines.push(String::new());
        lines.push(String::from("Created:"));
        for path in &report.created {
            lines.push(format!("  {}", path.display()));
        }
    }

    if !report.updated.is_empty() {
        lines.push(String::new());
        lines.push(String::from("Updated:"));
        for path in &report.updated {
            lines.push(format!("  {}", path.display()));
        }
    }

    if !report.replaced.is_empty() {
        lines.push(String::new());
        lines.push(String::from("Replaced:"));
        for path in &report.replaced {
            lines.push(format!("  {}", path.display()));
        }
    }

    if !report.skipped.is_empty() {
        lines.push(String::new());
        lines.push(String::from("Skipped:"));
        for path in &report.skipped {
            lines.push(format!("  {}", path.display()));
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
        lines.push(format!("  cd {}", report.project_path.display()));
    }
    lines.push(String::from("  moth check ."));
    lines.push(String::from("  moth dev ."));

    lines.join("\n")
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
