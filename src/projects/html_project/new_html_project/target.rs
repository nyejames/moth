//! Target path resolution for `moth new html`.
//!
//! WHAT: Resolves user-provided paths into canonical project directories with
//! interactive prompts for ambiguous or destructive placements.
//! WHY: Path semantics (omitted, relative, absolute, home expansion) and user
//! confirmation belong in one place, separate from file writing.

use std::path::{Path, PathBuf};

use crate::projects::html_project::new_html_project::prompt::Prompt;

/// Fully resolved project placement after all user interaction.
#[derive(Debug)]
pub struct ResolvedProjectTarget {
    pub project_dir: PathBuf,
    pub project_name: String,
    pub target_was_non_empty: bool,
}

/// Resolve the final project directory and name from CLI input and user prompts.
pub fn resolve_project_target(
    raw_path: Option<String>,
    current_dir: &Path,
    prompt: &mut impl Prompt,
) -> Result<ResolvedProjectTarget, String> {
    let resolved_dir = resolve_project_dir(raw_path, current_dir, prompt, &ProcessEnv)?;

    let target_exists = resolved_dir.exists();
    let target_was_non_empty = target_exists && is_directory_non_empty(&resolved_dir);

    let default_name = resolved_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Moth Project")
        .to_owned();

    let name_input = prompt.ask(&format!(
        "Project name (press Enter to use {default_name}): "
    ))?;
    let project_name = package_identifier_from_display_name(if name_input.trim().is_empty() {
        &default_name
    } else {
        name_input.trim()
    });

    Ok(ResolvedProjectTarget {
        project_dir: resolved_dir,
        project_name,
        target_was_non_empty,
    })
}

fn resolve_project_dir(
    raw_path: Option<String>,
    current_dir: &Path,
    prompt: &mut impl Prompt,
    env: &impl HomeEnv,
) -> Result<PathBuf, String> {
    match raw_path {
        None => {
            let message = format!(
                "No project path specified. Current directory: {}\n\
                 Create the new HTML project in this directory? [y/N]: ",
                current_dir.display()
            );
            if !prompt.confirm(&message, false)? {
                return Err("Cancelled project creation.".to_string());
            }
            Ok(current_dir.to_path_buf())
        }
        Some(path) if path == "." => Ok(current_dir.to_path_buf()),
        Some(path) => {
            let expanded = expand_tilde(&path, env)?;
            let resolved = if expanded.is_absolute() {
                normalize_path(&expanded)
            } else {
                normalize_path(&current_dir.join(expanded))
            };

            if resolved.exists() {
                handle_existing_directory(&resolved, prompt)
            } else {
                handle_missing_directory(&resolved, prompt)
            }
        }
    }
}

fn handle_existing_directory(path: &Path, prompt: &mut impl Prompt) -> Result<PathBuf, String> {
    let message = format!(
        "Target directory already exists:\n\
         {}\n\n\
         What do you want to do?\n\
           1. Create the project inside this directory\n\
           2. Create a new child folder for the project inside this directory\n\
           3. Cancel\n\n\
         Choose [1/2/3]: ",
        path.display()
    );

    loop {
        let choice = prompt.ask(&message)?;
        match choice.trim() {
            "1" => return Ok(path.to_path_buf()),
            "2" => {
                let folder_name = prompt.ask("Project folder name: ")?;
                let trimmed = folder_name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                return Ok(path.join(trimmed));
            }
            "3" => return Err("Cancelled project creation.".to_string()),
            _ => continue,
        }
    }
}

fn handle_missing_directory(path: &Path, prompt: &mut impl Prompt) -> Result<PathBuf, String> {
    let message = format!(
        "The project target contains directories that do not exist:\n\
         {}\n\n\
         Create the missing directories and scaffold the project there? [y/N]: ",
        path.display()
    );

    if !prompt.confirm(&message, false)? {
        return Err("Cancelled project creation.".to_string());
    }

    Ok(path.to_path_buf())
}

/// Isolated environment-variable lookup for home-directory resolution.
///
/// WHAT: Reads the process environment through a narrow trait so that
/// `expand_tilde` can be tested with an injected mock instead of mutating
/// process-global variables.
/// WHY: Direct `std::env::var("HOME")` inside `expand_tilde` forced tests to
/// set and restore `HOME` globally, which races under parallel test execution
/// and cannot exercise Windows fallback variables on a Unix host.
pub(super) trait HomeEnv {
    fn get(&self, key: &str) -> Option<String>;
    fn is_windows(&self) -> bool;
}

/// Production `HomeEnv` backed by the real process environment.
struct ProcessEnv;

impl HomeEnv for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn is_windows(&self) -> bool {
        cfg!(windows)
    }
}

/// Resolve the current user's home directory.
///
/// Tries `HOME` first. On Windows, it then falls back to `USERPROFILE` and a
/// complete `HOMEDRIVE` plus `HOMEPATH` pair. Empty values are treated as unset.
fn resolve_home(env: &impl HomeEnv) -> Result<PathBuf, String> {
    if let Some(home) = non_blank_env(env, "HOME") {
        return Ok(PathBuf::from(home));
    }

    if env.is_windows() {
        if let Some(userprofile) = non_blank_env(env, "USERPROFILE") {
            return Ok(PathBuf::from(userprofile));
        }
        if let (Some(drive), Some(path)) = (
            non_blank_env(env, "HOMEDRIVE"),
            non_blank_env(env, "HOMEPATH"),
        ) {
            return Ok(PathBuf::from(format!("{drive}{path}")));
        }
    }

    Err("Could not determine home directory for '~' expansion.".to_string())
}

/// Return a non-blank environment value, treating empty strings as unset.
fn non_blank_env(env: &impl HomeEnv, key: &str) -> Option<String> {
    env.get(key).filter(|value| !value.is_empty())
}

/// Expand a leading tilde into the current user's home directory.
///
/// Only bare `~`, `~/...` and `~\...` expand. Forms such as `~other` or
/// `~other/...` are left unchanged so named-user shorthand is not misread as
/// the current user's home. Windows backslash separators in the remainder are
/// normalised to forward slashes so the result resolves to the same logical
/// components as the slash-separated form on any host platform.
pub(super) fn expand_tilde(path: &str, env: &impl HomeEnv) -> Result<PathBuf, String> {
    let Some(rest) = path.strip_prefix('~') else {
        return Ok(PathBuf::from(path));
    };

    let is_bare = rest.is_empty();
    let is_slash_separated = rest.starts_with('/');
    let is_backslash_separated = rest.starts_with('\\');

    if !is_bare && !is_slash_separated && !is_backslash_separated {
        return Ok(PathBuf::from(path));
    }

    let home = resolve_home(env)?;

    if is_bare {
        return Ok(home);
    }

    let remainder = rest[1..].replace('\\', "/");
    if remainder.is_empty() {
        Ok(home)
    } else {
        Ok(home.join(remainder))
    }
}

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => result.push(prefix.as_os_str()),
            std::path::Component::RootDir => result.push("/"),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if result.file_name().is_some() {
                    result.pop();
                }
            }
            std::path::Component::Normal(name) => result.push(name),
        }
    }

    result
}

/// Convert a directory or prompt name into a `project.name` package identifier.
///
/// WHAT: keeps letters, digits and underscores; every other run of characters
/// becomes one underscore. Names that would start with a digit or be empty
/// after sanitising get a `project` prefix.
/// WHY: `config.moth` requires `project.name` to be a valid Moth project identifier,
/// so `moth new` must not write hyphenated or spaced names that fail the first build.
pub(super) fn package_identifier_from_display_name(name: &str) -> String {
    let mut identifier = String::new();
    let mut pending_separator = false;

    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            if pending_separator && !identifier.is_empty() {
                identifier.push('_');
            }
            pending_separator = false;
            identifier.push(character);
        } else {
            pending_separator = true;
        }
    }

    if identifier.is_empty()
        || identifier
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
    {
        identifier.insert_str(0, "project_");
    }

    identifier
}

fn is_directory_non_empty(path: &Path) -> bool {
    if let Ok(mut entries) = std::fs::read_dir(path) {
        entries.next().is_some()
    } else {
        false
    }
}
