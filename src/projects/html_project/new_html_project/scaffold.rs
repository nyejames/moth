//! File-writing scaffold logic.
//!
//! WHAT: Performs preflight conflict checks and actual directory/file creation.
//! WHY: Separates IO side effects from command parsing and user prompting.

use std::fs;
use std::path::{Path, PathBuf};

use crate::projects::html_project::new_html_project::{
    CreateProjectReport, prompt::Prompt, start_page_scaffolding, target::ResolvedProjectTarget,
};
use crate::projects::settings::CONFIG_FILE_NAME;

const PAGE_FILE: &str = "src/#page.moth";
const DEV_MANIFEST: &str = "dev/.moth_manifest";
const RELEASE_MANIFEST: &str = "release/.moth_manifest";
const GITIGNORE_FILE: &str = ".gitignore";

/// Relative paths of all scaffold-owned files.
const SCAFFOLD_OWNED_FILES: &[&str] =
    &[CONFIG_FILE_NAME, PAGE_FILE, DEV_MANIFEST, RELEASE_MANIFEST];

/// Relative paths of all scaffold-owned directories.
pub const SCAFFOLD_DIRECTORIES: &[&str] = &["src", "lib", "dev", "release"];

/// Run preflight checks before writing anything.
///
/// Returns Ok(()) when it is safe to proceed, or Err when the user cancels
/// or when unresolvable conflicts exist without `--force`.
pub(crate) fn run_preflight_checks(
    target: &ResolvedProjectTarget,
    force: bool,
    prompt: &mut impl Prompt,
) -> Result<(), String> {
    let conflicts = find_scaffold_conflicts(&target.project_dir);

    if !conflicts.is_empty() && !force {
        let file_list = conflicts
            .iter()
            .map(|path| format!("  {path}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "Cannot create project because scaffold-owned files already exist:\n\
             {file_list}\n\n\
             Run with --force to replace scaffold-owned files."
        ));
    }

    if !conflicts.is_empty() && force {
        let file_list = conflicts
            .iter()
            .map(|path| format!("  {path}"))
            .collect::<Vec<_>>()
            .join("\n");
        let message = format!(
            "WARNING: --force will overwrite existing Moth scaffold files in:\n\
             {}\n\n\
             Files that may be replaced:\n\
             {file_list}\n\n\
             Continue? [y/N]: ",
            target.project_dir.display()
        );
        if !prompt.confirm(&message, false)? {
            return Err("Cancelled project creation.".to_string());
        }
    }

    if target.target_was_non_empty && conflicts.is_empty() {
        let message = format!(
            "WARNING: The target directory is not empty:\n\
             {}\n\n\
             Creating a project here may mix scaffold files with existing content.\n\n\
             Continue? [y/N]: ",
            target.project_dir.display()
        );
        if !prompt.confirm(&message, false)? {
            return Err("Cancelled project creation.".to_string());
        }
    }

    Ok(())
}

pub fn find_scaffold_conflicts(project_dir: &Path) -> Vec<&'static str> {
    SCAFFOLD_OWNED_FILES
        .iter()
        .copied()
        .filter(|relative| project_dir.join(relative).exists())
        .collect()
}

/// Write the complete scaffold to disk.
///
/// WHAT: creates directories, writes scaffold-owned files, and handles `.gitignore`
/// creation or append after confirming with the user.
/// WHY: this is the single IO entry point for the scaffold; all file writes are
/// ordered and tracked so the caller can report exactly what happened.
pub(crate) fn write_scaffold(
    target: &ResolvedProjectTarget,
    force: bool,
    prompt: &mut impl Prompt,
) -> Result<CreateProjectReport, String> {
    let project_dir = &target.project_dir;

    // 1. Create missing directories (including the project dir itself).
    if !project_dir.exists() {
        fs::create_dir_all(project_dir).map_err(|e| {
            format!(
                "Project creation failed while creating directory {}: {e}.",
                project_dir.display()
            )
        })?;
    }

    let mut created: Vec<PathBuf> = Vec::new();
    let mut replaced: Vec<PathBuf> = Vec::new();

    for dir in SCAFFOLD_DIRECTORIES {
        let dir_path = project_dir.join(dir);
        if !dir_path.exists() {
            fs::create_dir_all(&dir_path).map_err(|e| {
                format!(
                    "Project creation failed while creating directory {}: {e}.",
                    dir_path.display()
                )
            })?;
            created.push(PathBuf::from(dir));
        }
    }

    // Helper to write a scaffold-owned file and track its disposition.
    let mut write_scaffold_file = |relative: &str, content: &str| -> Result<(), String> {
        let path = project_dir.join(relative);
        let existed = path.exists();
        fs::write(&path, content).map_err(|e| {
            format!(
                "Project creation failed while writing {relative}: {e}.\n\
                 Some scaffold directories may already have been created. \
                 No existing files were overwritten unless --force was confirmed."
            )
        })?;
        if existed && force {
            replaced.push(PathBuf::from(relative));
        } else {
            created.push(PathBuf::from(relative));
        }
        Ok(())
    };

    // 2. Config file.
    write_scaffold_file(
        CONFIG_FILE_NAME,
        &start_page_scaffolding::config_template(&target.project_name),
    )?;

    // 3. Starter page.
    write_scaffold_file(PAGE_FILE, start_page_scaffolding::page_template())?;

    // 4. Dev manifest.
    write_scaffold_file(DEV_MANIFEST, start_page_scaffolding::manifest_template())?;

    // 5. Release manifest.
    write_scaffold_file(
        RELEASE_MANIFEST,
        start_page_scaffolding::manifest_template(),
    )?;

    // 6. .gitignore (handled separately — never overwritten).
    let gitignore = handle_gitignore(project_dir, prompt)?;

    if let Some(path) = gitignore.created {
        created.push(path);
    }

    Ok(CreateProjectReport {
        project_path: project_dir.clone(),
        project_name: target.project_name.clone(),
        created,
        updated: gitignore.updated.into_iter().collect(),
        skipped: gitignore.skipped.into_iter().collect(),
        replaced,
    })
}

/// Result of handling `.gitignore` during scaffold creation.
struct GitignoreResult {
    created: Option<PathBuf>,
    updated: Option<PathBuf>,
    skipped: Option<PathBuf>,
}

/// Handle `.gitignore` creation or append.
///
/// Returns a `GitignoreResult` where at most one field is Some.
fn handle_gitignore(
    project_dir: &Path,
    prompt: &mut impl Prompt,
) -> Result<GitignoreResult, String> {
    let gitignore_path = project_dir.join(GITIGNORE_FILE);

    if !gitignore_path.exists() {
        let should_create = prompt.confirm("Add a .gitignore with Moth defaults? [Y/n]: ", true)?;
        if should_create {
            fs::write(
                &gitignore_path,
                start_page_scaffolding::gitignore_template(),
            )
            .map_err(|e| {
                format!(
                    "Project creation failed while writing .gitignore: {e}.\n\
                     Some scaffold directories may already have been created. \
                     No existing files were overwritten unless --force was confirmed."
                )
            })?;
            return Ok(GitignoreResult {
                created: Some(PathBuf::from(GITIGNORE_FILE)),
                updated: None,
                skipped: None,
            });
        }
        return Ok(GitignoreResult {
            created: None,
            updated: None,
            skipped: Some(PathBuf::from(GITIGNORE_FILE)),
        });
    }

    // Existing .gitignore — check whether it already contains the Moth block.
    let existing = fs::read_to_string(&gitignore_path).map_err(|e| {
        format!(
            "Project creation failed while reading existing .gitignore: {e}.\n\
             Some scaffold directories may already have been created. \
             No existing files were overwritten unless --force was confirmed."
        )
    })?;

    if existing_contains_dev_block(&existing) {
        return Ok(GitignoreResult {
            created: None,
            updated: None,
            skipped: Some(PathBuf::from(GITIGNORE_FILE)),
        });
    }

    let should_append = prompt.confirm(
        ".gitignore already exists.\nAdd missing Moth defaults to it? [Y/n]: ",
        true,
    )?;
    if should_append {
        let mut appended = existing;
        if !appended.ends_with('\n') {
            appended.push('\n');
        }
        appended.push_str(start_page_scaffolding::gitignore_append_block());
        fs::write(&gitignore_path, appended).map_err(|e| {
            format!(
                "Project creation failed while appending to .gitignore: {e}.\n\
                 Some scaffold directories may already have been created. \
                 No existing files were overwritten unless --force was confirmed."
            )
        })?;
        return Ok(GitignoreResult {
            created: None,
            updated: Some(PathBuf::from(GITIGNORE_FILE)),
            skipped: None,
        });
    }

    Ok(GitignoreResult {
        created: None,
        updated: None,
        skipped: Some(PathBuf::from(GITIGNORE_FILE)),
    })
}

/// Check whether an existing `.gitignore` already contains the Moth `/dev` rule.
///
/// Recognizes a trimmed line equal to `/dev` or `/dev/` so that a trailing
/// slash does not cause a duplicate append. Uses exact line equality after
/// trimming, not substring matching, so near matches such as `/device`,
/// `prefix/dev`, `/dev/**` and commented rules are not treated as present.
pub(super) fn existing_contains_dev_block(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "/dev" || trimmed == "/dev/"
    })
}
