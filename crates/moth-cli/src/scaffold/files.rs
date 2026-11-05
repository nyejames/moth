//! File-writing scaffold logic.
//!
//! WHAT: Performs preflight conflict checks and actual directory/file creation.
//! WHY: Separates IO side effects from command parsing and user prompting.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::scaffold::{
    CreateProjectReport, prompt::Prompt, start_page_scaffolding, target::ResolvedProjectTarget,
};
use moth::projects::settings::CONFIG_FILE_NAME;
use tempfile::NamedTempFile;

const PAGE_FILE: &str = "@page.moth";
const GITIGNORE_FILE: &str = ".gitignore";

/// Relative paths of all scaffold-owned files.
const SCAFFOLD_OWNED_FILES: &[&str] = &[CONFIG_FILE_NAME, PAGE_FILE];

/// The filesystem shape of one entry observed without following its final symlink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScaffoldEntryKind {
    RegularFile,
    Symlink,
    Directory,
    Other,
}

impl ScaffoldEntryKind {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            Self::Symlink
        } else if file_type.is_file() {
            Self::RegularFile
        } else if file_type.is_dir() {
            Self::Directory
        } else {
            Self::Other
        }
    }
}

/// One scaffold-owned path that already exists in the target directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScaffoldConflict {
    pub(crate) relative: &'static str,
    pub(crate) kind: ScaffoldEntryKind,
}

/// Run preflight checks before writing anything.
///
/// Returns Ok(()) when it is safe to proceed, or Err for an unsupported scaffold-owned entry
/// kind, a non-regular `.gitignore`, a regular-file conflict without `--force`, or a declined
/// confirmation prompt.
pub(crate) fn run_preflight_checks(
    target: &ResolvedProjectTarget,
    force: bool,
    prompt: &mut impl Prompt,
) -> Result<(), String> {
    let conflicts = find_scaffold_conflicts(&target.project_dir)?;

    reject_unsupported_conflicts(&conflicts, ScaffoldEntryKind::Symlink)?;
    reject_unsupported_conflicts(&conflicts, ScaffoldEntryKind::Directory)?;
    reject_unsupported_conflicts(&conflicts, ScaffoldEntryKind::Other)?;
    reject_unsupported_gitignore(&target.project_dir)?;
    validate_existing_gitignore(&target.project_dir)?;

    let conflict_names = conflicts
        .iter()
        .map(|conflict| conflict.relative)
        .collect::<Vec<_>>();

    if !conflict_names.is_empty() && !force {
        let file_list = conflict_names
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

    if !conflict_names.is_empty() && force {
        let file_list = conflict_names
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

    if target.target_was_non_empty && conflict_names.is_empty() {
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

fn reject_unsupported_conflicts(
    conflicts: &[ScaffoldConflict],
    kind: ScaffoldEntryKind,
) -> Result<(), String> {
    if kind == ScaffoldEntryKind::RegularFile {
        return Ok(());
    }
    let paths = conflicts
        .iter()
        .filter(|conflict| conflict.kind == kind)
        .map(|conflict| conflict.relative)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok(());
    }

    let file_list = paths
        .iter()
        .map(|path| format!("  {path}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (category, action) = match kind {
        ScaffoldEntryKind::Symlink => (
            "symbolic links",
            "Replace them with regular files. --force does not replace symbolic links.",
        ),
        ScaffoldEntryKind::Directory => (
            "directories",
            "Remove them or replace them with regular files. --force only replaces regular files.",
        ),
        ScaffoldEntryKind::Other => (
            "unsupported filesystem entries",
            "Remove them or replace them with regular files. --force only replaces regular files.",
        ),
        ScaffoldEntryKind::RegularFile => unreachable!("regular files are not rejected"),
    };
    Err(format!(
        "Cannot create project because scaffold-owned paths are {category}:\n\
         {file_list}\n\n\
         {action}"
    ))
}

fn reject_unsupported_gitignore(project_dir: &Path) -> Result<(), String> {
    let Some(metadata) = entry_metadata(&project_dir.join(GITIGNORE_FILE), GITIGNORE_FILE)? else {
        return Ok(());
    };
    let kind = ScaffoldEntryKind::from_metadata(&metadata);
    if kind == ScaffoldEntryKind::RegularFile {
        return Ok(());
    }
    Err(format!(
        "Cannot create project because .gitignore is {}. Remove it or replace it with a \
         regular file; --force only replaces regular files.",
        entry_kind_noun(kind),
    ))
}

/// Validate an existing regular `.gitignore` as UTF-8 before any scaffold writes.
///
/// WHY: a late UTF-8 or read failure would otherwise leave a partly written project even when the
/// failure was present from the start. Preflight handles deterministic failures only; IO errors
/// and concurrent modifications can still occur after writing begins.
fn validate_existing_gitignore(project_dir: &Path) -> Result<(), String> {
    let path = project_dir.join(GITIGNORE_FILE);
    let Some(metadata) = entry_metadata(&path, GITIGNORE_FILE)? else {
        return Ok(());
    };
    // `reject_unsupported_gitignore` already rejected every non-regular entry.
    if ScaffoldEntryKind::from_metadata(&metadata) != ScaffoldEntryKind::RegularFile {
        return Ok(());
    }

    fs::read_to_string(&path).map_err(|error| {
        format!(
            "Project creation failed while reading existing .gitignore: {error}. \
             It must be readable UTF-8 text; no scaffold files were written."
        )
    })?;
    Ok(())
}

fn entry_kind_noun(kind: ScaffoldEntryKind) -> &'static str {
    match kind {
        ScaffoldEntryKind::Symlink => "a symbolic link",
        ScaffoldEntryKind::Directory => "a directory",
        ScaffoldEntryKind::Other => "an unsupported filesystem entry",
        ScaffoldEntryKind::RegularFile => "a regular file",
    }
}

/// Report each scaffold-owned path that exists, without following a final symlink.
///
/// The result retains the observed kind so preflight can reject symlinks, directories and other
/// unsupported entries before prompting. A missing entry is not a conflict. Filesystem errors are
/// returned rather than treated as an absent file.
pub(crate) fn find_scaffold_conflicts(project_dir: &Path) -> Result<Vec<ScaffoldConflict>, String> {
    let mut conflicts = Vec::new();
    for relative in SCAFFOLD_OWNED_FILES {
        let path = project_dir.join(relative);
        let Some(metadata) = entry_metadata(&path, relative)? else {
            continue;
        };
        conflicts.push(ScaffoldConflict {
            relative,
            kind: ScaffoldEntryKind::from_metadata(&metadata),
        });
    }
    Ok(conflicts)
}

/// Read one path's metadata without following a final symlink.
///
/// `Ok(None)` distinguishes a missing entry from filesystem errors, which stay visible instead of
/// being reported as a clean target.
fn entry_metadata(path: &Path, relative: &str) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Project creation failed while inspecting {relative}: {error}."
        )),
    }
}

fn unsupported_scaffold_entry_error(relative: &str, kind: ScaffoldEntryKind) -> String {
    format!(
        "Project creation failed because {relative} is {}. Replace it with a regular file.",
        entry_kind_noun(kind),
    )
}

fn existing_scaffold_file_needs_force(relative: &str) -> String {
    format!(
        "Project creation failed because {relative} already exists. \
         Run with --force to replace an existing regular scaffold file."
    )
}

/// Create a file that must not already exist.
///
/// `create_new` refuses an existing final entry, including a symlink introduced after the
/// metadata preflight, so this creation path cannot silently reuse a path that appeared later.
fn write_new_file(path: &Path, content: &str, relative: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Project creation failed while creating {relative}: {error}."))?;
    file.write_all(content.as_bytes())
        .map_err(|error| format!("Project creation failed while writing {relative}: {error}."))
}

/// Stage complete replacement bytes beside the destination and publish them by rename.
///
/// WHAT: replaces only an existing regular file whose final entry still has that kind.
/// WHY: writing in place would truncate the original on failure and modify every other hard link
/// to its inode. Renaming a completed same-directory file replaces the final directory entry
/// without writing through the old inode or a final symlink.
///
/// This is per-file publication, not a transaction across config/page/.gitignore and not a
/// power-loss durability guarantee. Last-writer-wins races and parent-path substitution require a
/// different concurrency/security model.
fn replace_existing_regular_file(path: &Path, content: &str, relative: &str) -> Result<(), String> {
    replace_existing_regular_file_inner(path, content, relative, |staged| {
        staged
            .persist(path)
            .map(|_| ())
            .map_err(|error| error.error)
    })
}

fn replace_existing_regular_file_inner(
    path: &Path,
    content: &str,
    relative: &str,
    publish: impl FnOnce(NamedTempFile) -> std::io::Result<()>,
) -> Result<(), String> {
    let Some(metadata) = entry_metadata(path, relative)? else {
        return Err(format!(
            "Project creation failed because {relative} disappeared before it could be replaced."
        ));
    };
    let kind = ScaffoldEntryKind::from_metadata(&metadata);
    if kind != ScaffoldEntryKind::RegularFile {
        return Err(unsupported_scaffold_entry_error(relative, kind));
    }

    let parent = path.parent().ok_or_else(|| {
        format!("Project creation failed because {relative} has no parent directory.")
    })?;
    let mut staged = NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Project creation failed while staging a replacement for {relative}: {error}. \
             The existing file was not replaced."
        )
    })?;
    staged
        .as_file_mut()
        .write_all(content.as_bytes())
        .map_err(|error| {
            format!(
                "Project creation failed while writing a staged replacement for {relative}: \
                 {error}. The existing file was not replaced."
            )
        })?;
    // Preserve ordinary permissions only; ownership, ACLs and extended attributes are not copied.
    staged
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(|error| {
            format!(
                "Project creation failed while setting permissions on a staged replacement for \
                 {relative}: {error}. The existing file was not replaced."
            )
        })?;
    publish(staged).map_err(|error| {
        format!(
            "Project creation failed while publishing the replacement for {relative}: {error}. \
             The existing file was not replaced."
        )
    })?;
    Ok(())
}

/// Exercise a publication failure after the staged file is complete.
///
/// The staging and permission paths are production code; only the final publish step is injected.
#[cfg(test)]
pub(super) fn replace_existing_regular_file_with_failed_publish_for_test(
    path: &Path,
    content: &str,
    relative: &str,
) -> Result<(), String> {
    replace_existing_regular_file_inner(path, content, relative, |_staged| {
        Err(std::io::Error::other("injected publish failure"))
    })
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
    let config_contents = start_page_scaffolding::config_template(&target.project_name)?;
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

    // Preflight handles deterministic failures. IO errors and concurrent modifications can still
    // occur after writing begins; this command does not promise rollback across files.
    let mut write_scaffold_file = |relative: &str, content: &str| -> Result<(), String> {
        let path = project_dir.join(relative);
        let metadata = entry_metadata(&path, relative)?;
        let Some(metadata) = metadata else {
            // create_new preserves the no-overwrite contract if a file appears after preflight.
            // Do not silently use replacement for paths that were absent.
            write_new_file(&path, content, relative)?;
            created.push(PathBuf::from(relative));
            return Ok(());
        };

        let kind = ScaffoldEntryKind::from_metadata(&metadata);
        if kind != ScaffoldEntryKind::RegularFile {
            return Err(unsupported_scaffold_entry_error(relative, kind));
        }
        if !force {
            return Err(existing_scaffold_file_needs_force(relative));
        }

        // --force permits replacing an existing regular scaffold file, not writing into its
        // current inode. Keep this on the atomic replacement path to protect hard-linked files
        // and retain the original contents if staging fails.
        replace_existing_regular_file(&path, content, relative)?;
        replaced.push(PathBuf::from(relative));
        Ok(())
    };

    // 2. Config file.
    write_scaffold_file(CONFIG_FILE_NAME, &config_contents)?;

    // 3. Starter page.
    write_scaffold_file(PAGE_FILE, start_page_scaffolding::page_template())?;

    // 4. .gitignore (handled separately; --force never replaces it).
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

    let metadata = entry_metadata(&gitignore_path, GITIGNORE_FILE)?;
    let Some(metadata) = metadata else {
        let should_create = prompt.confirm("Add a .gitignore with Moth defaults? [Y/n]: ", true)?;
        if should_create {
            write_new_file(
                &gitignore_path,
                start_page_scaffolding::gitignore_template(),
                GITIGNORE_FILE,
            )
            .map_err(|error| {
                format!("{error}\nSome scaffold files may already have been written.")
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
    };

    let kind = ScaffoldEntryKind::from_metadata(&metadata);
    if kind != ScaffoldEntryKind::RegularFile {
        return Err(unsupported_scaffold_entry_error(GITIGNORE_FILE, kind));
    }

    // Existing .gitignore — check whether it already contains the Moth block.
    let existing = fs::read_to_string(&gitignore_path).map_err(|error| {
        format!(
            "Project creation failed while reading existing .gitignore: {error}.\n\
             Some scaffold files may already have been written."
        )
    })?;

    if existing_contains_moth_output_block(&existing) {
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
        let Some(metadata) = entry_metadata(&gitignore_path, GITIGNORE_FILE)? else {
            return Err(
                "Project creation failed because .gitignore disappeared after the append was \
                 confirmed.\nSome scaffold files may already have been written."
                    .to_string(),
            );
        };
        let kind = ScaffoldEntryKind::from_metadata(&metadata);
        if kind != ScaffoldEntryKind::RegularFile {
            return Err(unsupported_scaffold_entry_error(GITIGNORE_FILE, kind));
        }

        // Re-read the current text before publishing the append so content changed while the user
        // decided is not overwritten using an older snapshot. This narrows, but does not
        // eliminate, concurrent-edit races.
        let current = fs::read_to_string(&gitignore_path).map_err(|error| {
            format!(
                "Project creation failed while re-reading existing .gitignore: {error}.\n\
                 Some scaffold files may already have been written."
            )
        })?;
        if existing_contains_moth_output_block(&current) {
            return Ok(GitignoreResult {
                created: None,
                updated: None,
                skipped: Some(PathBuf::from(GITIGNORE_FILE)),
            });
        }

        let mut appended = current;
        let block = moth_output_append_block(&appended);
        if !appended.ends_with('\n') {
            appended.push('\n');
        }
        if !appended.ends_with("\n\n") {
            appended.push('\n');
        }
        appended.push_str(&block);

        // An append is prepared as complete replacement text and published by rename for the same
        // reason as forced scaffold replacement. The append prompt does not authorize a
        // truncate-on-write update.
        replace_existing_regular_file(&gitignore_path, &appended, GITIGNORE_FILE).map_err(
            |error| format!("{error}\nSome scaffold files may already have been written."),
        )?;
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

/// Check whether an existing `.gitignore` already contains the Moth output rules.
///
/// Both `/_dev` and `/_release` must be present. Each rule is recognized as a
/// trimmed line equal to itself optionally followed by a trailing slash, so a
/// trailing slash does not cause a duplicate append. Uses exact line equality
/// after trimming, not substring matching, so near matches such as `/_device`,
/// `prefix/_dev`, `/_dev/**` and commented rules are not treated as present.
pub(super) fn existing_contains_moth_output_block(content: &str) -> bool {
    contains_moth_output_rule(content, "/_dev") && contains_moth_output_rule(content, "/_release")
}

fn contains_moth_output_rule(content: &str, rule: &str) -> bool {
    let with_slash = format!("{rule}/");
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == rule || trimmed == with_slash
    })
}

/// Build the append block containing only rules that are still missing.
///
/// WHAT: emits the Moth header once and only absent output rules.
/// WHY: appending the whole block when one rule already exists duplicates that rule.
fn moth_output_append_block(existing: &str) -> String {
    let mut block = String::new();
    if !existing
        .lines()
        .any(|line| line.trim() == "# Moth generated output")
    {
        block.push_str("# Moth generated output\n");
    }
    for rule in ["/_dev", "/_release"] {
        if !contains_moth_output_rule(existing, rule) {
            block.push_str(rule);
            block.push('\n');
        }
    }
    block
}
