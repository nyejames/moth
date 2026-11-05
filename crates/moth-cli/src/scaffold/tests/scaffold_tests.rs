//! Tests for the `moth new html` file-writing scaffold.
//!
//! Each test owns one distinct scaffold contract. Same-family decision pairs
//! and inventories share one labelled owner so failure localization stays clear,
//! while the direct `write_scaffold` IO policy is kept separate from the
//! end-to-end `create_html_project_template_with_prompt` orchestration that
//! protects the no-write and force command boundaries.

use crate::scaffold::files::{
    ScaffoldEntryKind, existing_contains_moth_output_block, find_scaffold_conflicts,
    replace_existing_regular_file_with_failed_publish_for_test, run_preflight_checks,
    write_scaffold,
};
use crate::scaffold::prompt::Prompt;
use crate::scaffold::prompt_tests::ScriptedPrompt;
use crate::scaffold::start_page_scaffolding;
use crate::scaffold::target::ResolvedProjectTarget;
use crate::scaffold::{NewHtmlProjectOptions, create_html_project_template_with_prompt};
use std::fs;
use std::path::PathBuf;

fn empty_target(path: PathBuf) -> ResolvedProjectTarget {
    ResolvedProjectTarget {
        project_dir: path,
        project_name: String::from("test"),
        target_was_non_empty: false,
    }
}

fn named_target(
    project_dir: PathBuf,
    project_name: &str,
    non_empty: bool,
) -> ResolvedProjectTarget {
    ResolvedProjectTarget {
        project_dir,
        project_name: String::from(project_name),
        target_was_non_empty: non_empty,
    }
}

/// Test prompt that mutates the filesystem on its first confirmation.
///
/// This deterministically stages a late `.gitignore` disappearance or replacement after the user
/// has confirmed an append, without timing-dependent concurrency.
struct MutatingConfirmPrompt {
    mutation: Option<Box<dyn FnOnce()>>,
    messages: Vec<String>,
}

impl MutatingConfirmPrompt {
    fn new(mutation: impl FnOnce() + 'static) -> Self {
        Self {
            mutation: Some(Box::new(mutation)),
            messages: Vec::new(),
        }
    }
}

impl Prompt for MutatingConfirmPrompt {
    fn ask(&mut self, _message: &str) -> Result<String, String> {
        Err("MutatingConfirmPrompt does not script open-ended prompts".to_string())
    }

    fn confirm(&mut self, message: &str, _default: bool) -> Result<bool, String> {
        self.messages.push(message.to_owned());
        if let Some(mutation) = self.mutation.take() {
            mutation();
        }
        Ok(true)
    }
}

#[test]
fn find_scaffold_conflicts_reports_exact_owned_set_and_excludes_unowned_directories_and_gitignore()
{
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();

    // Scaffold-owned files are the exact conflict set, in declared order.
    for file in ["config.moth", "@page.moth"] {
        fs::write(project_dir.join(file), b"old").unwrap();
    }
    assert_eq!(
        find_scaffold_conflicts(&project_dir)
            .unwrap()
            .iter()
            .map(|conflict| conflict.relative)
            .collect::<Vec<_>>(),
        vec!["config.moth", "@page.moth",]
    );

    // Generated output directories and a user .gitignore are never conflicts.
    let clean = tempfile::tempdir().unwrap();
    let clean_dir = clean.path().to_path_buf();
    for dir in ["_dev", "_release"] {
        fs::create_dir_all(clean_dir.join(dir)).unwrap();
    }
    fs::write(clean_dir.join(".gitignore"), b"old").unwrap();
    assert!(find_scaffold_conflicts(&clean_dir).unwrap().is_empty());
}

#[test]
fn preflight_rejects_conflicts_without_force() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    fs::write(project_dir.join("config.moth"), b"old").unwrap();

    let target = empty_target(project_dir);
    let mut prompt = ScriptedPrompt::new(Vec::new());

    let error = run_preflight_checks(&target, false, &mut prompt).unwrap_err();

    assert!(error.contains("Cannot create project"));
    assert!(error.contains("config.moth"));
    assert!(error.contains("--force"));
}

#[test]
fn preflight_succeeds_without_prompt_when_no_conflicts_exist() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();

    let target = empty_target(project_dir);
    let mut prompt = ScriptedPrompt::new(Vec::new());

    assert!(run_preflight_checks(&target, false, &mut prompt).is_ok());
    assert!(prompt.messages.is_empty());
}

#[test]
fn preflight_force_confirmation_confirms_or_cancels() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    fs::write(project_dir.join("config.moth"), b"old").unwrap();

    // Confirming proceeds and the warning lists the conflicting file.
    let target = empty_target(project_dir.clone());
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);
    assert!(run_preflight_checks(&target, true, &mut prompt).is_ok());
    assert!(prompt.messages[0].contains("WARNING: --force"));
    assert!(prompt.messages[0].contains("config.moth"));

    // Declining cancels project creation.
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);
    let error = run_preflight_checks(&target, true, &mut prompt).unwrap_err();
    assert_eq!(error, "Cancelled project creation.");
}

#[cfg(unix)]
#[test]
fn preflight_rejects_dangling_scaffold_symlinks_even_with_force() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    let dangling_target = project_dir.join("missing-target");
    symlink(&dangling_target, project_dir.join("config.moth")).unwrap();

    let conflicts = find_scaffold_conflicts(&project_dir).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].relative, "config.moth");
    assert_eq!(conflicts[0].kind, ScaffoldEntryKind::Symlink);

    for force in [false, true] {
        let target = empty_target(project_dir.clone());
        let mut prompt = ScriptedPrompt::new(Vec::new());
        let error = run_preflight_checks(&target, force, &mut prompt).unwrap_err();
        assert!(
            error.contains("symbolic links") && error.contains("config.moth"),
            "force={force}: {error}"
        );
    }
    assert!(
        !dangling_target.exists(),
        "preflight must not create a dangling symlink target"
    );
}

#[cfg(unix)]
#[test]
fn write_scaffold_refuses_scaffold_symlinks_with_and_without_force() {
    use std::os::unix::fs::symlink;

    for force in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let outside = temp.path().join("outside.moth");
        fs::write(&outside, b"keep me").unwrap();
        symlink(&outside, project_dir.join("config.moth")).unwrap();

        let target = named_target(project_dir.clone(), "Test_Site", true);
        let mut prompt = ScriptedPrompt::new(Vec::new());

        let error = write_scaffold(&target, force, &mut prompt).unwrap_err();
        assert!(
            error.contains("config.moth") && error.contains("symbolic link"),
            "force={force}: {error}"
        );
        assert_eq!(
            fs::read(&outside).unwrap(),
            b"keep me",
            "the symlink target outside the project must stay untouched"
        );
        assert!(
            fs::symlink_metadata(project_dir.join("config.moth"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "the refused scaffold path must stay the authored symlink"
        );
    }
}

#[cfg(unix)]
#[test]
fn write_scaffold_rechecks_symlinks_after_preflight() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let target = named_target(project_dir.clone(), "Test_Site", false);

    let mut prompt = ScriptedPrompt::new(Vec::new());
    run_preflight_checks(&target, false, &mut prompt).unwrap();

    let outside = temp.path().join("outside.moth");
    fs::write(&outside, b"keep me").unwrap();
    symlink(&outside, project_dir.join("@page.moth")).unwrap();

    let mut prompt = ScriptedPrompt::new(Vec::new());
    let error = write_scaffold(&target, false, &mut prompt).unwrap_err();
    assert!(error.contains("@page.moth") && error.contains("symbolic link"));
    assert_eq!(fs::read(&outside).unwrap(), b"keep me");
}

#[cfg(unix)]
#[test]
fn write_scaffold_refuses_gitignore_symlinks() {
    use std::os::unix::fs::symlink;

    for dangling in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        let outside = temp.path().join("outside.gitignore");
        if !dangling {
            fs::write(&outside, b"custom\n").unwrap();
        }
        symlink(&outside, project_dir.join(".gitignore")).unwrap();

        let target = named_target(project_dir.clone(), "Test_Site", false);
        let mut prompt = ScriptedPrompt::new(Vec::new());

        let error = write_scaffold(&target, true, &mut prompt).unwrap_err();
        assert!(
            error.contains(".gitignore") && error.contains("symbolic link"),
            "dangling={dangling}: {error}"
        );
        if dangling {
            assert!(
                !outside.exists(),
                "a dangling .gitignore symlink must not create its target"
            );
        } else {
            assert_eq!(
                fs::read(&outside).unwrap(),
                b"custom\n",
                "an existing .gitignore target must stay untouched"
            );
        }
        assert!(
            fs::symlink_metadata(project_dir.join(".gitignore"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[test]
fn preflight_rejects_directory_scaffold_paths_even_with_force() {
    for relative in ["config.moth", "@page.moth"] {
        for force in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let project_dir = temp.path().to_path_buf();
            fs::create_dir(project_dir.join(relative)).unwrap();

            let target = empty_target(project_dir.clone());
            let mut prompt = ScriptedPrompt::new(Vec::new());
            let error = run_preflight_checks(&target, force, &mut prompt).unwrap_err();

            assert!(
                error.contains("directories") && error.contains(relative),
                "force={force}, path={relative}: {error}"
            );
            assert!(
                prompt.messages.is_empty(),
                "unsupported entry kinds must fail before confirmation prompts"
            );
            assert!(
                find_scaffold_conflicts(&project_dir)
                    .unwrap()
                    .iter()
                    .any(|conflict| conflict.relative == relative
                        && conflict.kind == ScaffoldEntryKind::Directory)
            );
        }
    }
}

#[test]
fn preflight_rejects_gitignore_directory_before_writes() {
    for force in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().to_path_buf();
        fs::create_dir(project_dir.join(".gitignore")).unwrap();

        let target = empty_target(project_dir.clone());
        let mut prompt = ScriptedPrompt::new(Vec::new());
        let error = run_preflight_checks(&target, force, &mut prompt).unwrap_err();

        assert!(
            error.contains(".gitignore") && error.contains("directory"),
            "force={force}: {error}"
        );
        assert!(!project_dir.join("config.moth").exists());
        assert!(!project_dir.join("@page.moth").exists());
    }
}

#[test]
fn preflight_treats_regular_gitignore_as_append_not_force_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    fs::write(project_dir.join(".gitignore"), b"custom\n").unwrap();

    let target = named_target(project_dir, "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);
    run_preflight_checks(&target, true, &mut prompt).unwrap();

    assert!(prompt.messages[0].contains("not empty"));
    assert!(
        prompt
            .messages
            .iter()
            .all(|message| !message.contains("--force")),
        "an existing regular .gitignore is an append decision, not a force replacement"
    );
}

#[cfg(unix)]
#[test]
fn preflight_rejects_gitignore_symlink_before_writes() {
    use std::os::unix::fs::symlink;

    for force in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().to_path_buf();
        let outside = temp.path().join("outside.gitignore");
        fs::write(&outside, b"custom\n").unwrap();
        symlink(&outside, project_dir.join(".gitignore")).unwrap();

        let target = empty_target(project_dir.clone());
        let mut prompt = ScriptedPrompt::new(Vec::new());
        let error = run_preflight_checks(&target, force, &mut prompt).unwrap_err();

        assert!(
            error.contains(".gitignore") && error.contains("symbolic link"),
            "force={force}: {error}"
        );
        assert_eq!(fs::read(&outside).unwrap(), b"custom\n");
        assert!(!project_dir.join("config.moth").exists());
        assert!(!project_dir.join("@page.moth").exists());
    }
}

#[test]
fn preflight_non_empty_warning_confirms_or_cancels() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    fs::write(project_dir.join("other.txt"), b"content").unwrap();

    let target = named_target(project_dir, "test", true);

    // Confirming proceeds after the non-empty warning.
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);
    assert!(run_preflight_checks(&target, false, &mut prompt).is_ok());
    assert!(prompt.messages[0].contains("not empty"));

    // Declining cancels project creation and performs no writes.
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);
    let error = run_preflight_checks(&target, false, &mut prompt).unwrap_err();
    assert_eq!(error, "Cancelled project creation.");
}

#[test]
fn end_to_end_conflict_without_force_performs_no_writes() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    fs::write(project_dir.join("config.moth"), b"original").unwrap();

    let options = NewHtmlProjectOptions {
        raw_path: Some(project_dir.to_string_lossy().to_string()),
        force: false,
    };
    let mut prompt = ScriptedPrompt::new(vec![String::from("1"), String::from("")]);

    let error = create_html_project_template_with_prompt(options, &mut prompt).unwrap_err();
    assert!(error.contains("Cannot create project"));

    let content = fs::read_to_string(project_dir.join("config.moth")).unwrap();
    assert_eq!(content, "original");
}

#[test]
fn end_to_end_directory_scaffold_paths_block_before_writes() {
    for relative in ["config.moth", "@page.moth"] {
        for force in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let project_dir = temp.path().to_path_buf();
            fs::create_dir(project_dir.join(relative)).unwrap();

            let options = NewHtmlProjectOptions {
                raw_path: Some(project_dir.to_string_lossy().to_string()),
                force,
            };
            let mut prompt = ScriptedPrompt::new(vec![String::from("1"), String::from("")]);

            let error = create_html_project_template_with_prompt(options, &mut prompt).unwrap_err();
            assert!(
                error.contains(relative) && error.contains("directories"),
                "force={force}, path={relative}: {error}"
            );
            assert!(project_dir.join(relative).is_dir());
            if relative == "config.moth" {
                assert!(
                    !project_dir.join("@page.moth").exists(),
                    "no later scaffold file may be written after a config.moth kind error"
                );
            } else {
                assert!(
                    !project_dir.join("config.moth").exists(),
                    "config.moth must not be created when @page.moth is a directory"
                );
            }
        }
    }
}

#[test]
fn end_to_end_gitignore_directory_blocks_before_scaffold_writes() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    fs::create_dir(project_dir.join(".gitignore")).unwrap();

    let options = NewHtmlProjectOptions {
        raw_path: Some(project_dir.to_string_lossy().to_string()),
        force: true,
    };
    let mut prompt = ScriptedPrompt::new(vec![String::from("1"), String::from("")]);

    let error = create_html_project_template_with_prompt(options, &mut prompt).unwrap_err();
    assert!(error.contains(".gitignore") && error.contains("directory"));
    assert!(!project_dir.join("config.moth").exists());
    assert!(!project_dir.join("@page.moth").exists());
    assert!(project_dir.join(".gitignore").is_dir());
}

#[cfg(unix)]
#[test]
fn end_to_end_gitignore_symlink_blocks_before_scaffold_writes() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    let outside = temp.path().join("outside.gitignore");
    fs::write(&outside, b"custom\n").unwrap();
    symlink(&outside, project_dir.join(".gitignore")).unwrap();

    let options = NewHtmlProjectOptions {
        raw_path: Some(project_dir.to_string_lossy().to_string()),
        force: true,
    };
    let mut prompt = ScriptedPrompt::new(vec![String::from("1"), String::from("")]);

    let error = create_html_project_template_with_prompt(options, &mut prompt).unwrap_err();
    assert!(error.contains(".gitignore") && error.contains("symbolic link"));
    assert_eq!(fs::read(&outside).unwrap(), b"custom\n");
    assert!(!project_dir.join("config.moth").exists());
    assert!(!project_dir.join("@page.moth").exists());
}

#[test]
fn end_to_end_non_utf8_gitignore_blocks_before_any_scaffold_writes() {
    for force in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().to_path_buf();
        let config = project_dir.join("config.moth");
        let page = project_dir.join("@page.moth");
        let gitignore = project_dir.join(".gitignore");
        fs::write(&config, b"old config").unwrap();
        fs::write(&page, b"old page").unwrap();
        let invalid = [0xff, 0xfe, b'\n'];
        fs::write(&gitignore, invalid).unwrap();

        let options = NewHtmlProjectOptions {
            raw_path: Some(project_dir.to_string_lossy().to_string()),
            force,
        };
        let mut prompt = ScriptedPrompt::new(vec![String::from("1"), String::from("")]);

        let error = create_html_project_template_with_prompt(options, &mut prompt).unwrap_err();
        assert!(
            error.contains(".gitignore") && error.contains("UTF-8"),
            "force={force}: {error}"
        );
        assert!(
            prompt.messages.iter().all(|message| {
                !message.contains(".gitignore")
                    && !message.contains("--force")
                    && !message.contains("not empty")
            }),
            "force={force}: invalid .gitignore must fail before append, force or non-empty prompts: \
             {:?}",
            prompt.messages
        );

        assert_eq!(fs::read(&config).unwrap(), b"old config");
        assert_eq!(fs::read(&page).unwrap(), b"old page");
        assert_eq!(
            fs::read(&gitignore).unwrap(),
            invalid,
            "invalid .gitignore bytes must stay byte-for-byte unchanged"
        );
    }
}

#[test]
fn end_to_end_force_overwrites_scaffold_owned_files_only() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();
    fs::write(project_dir.join("config.moth"), b"original").unwrap();
    fs::write(project_dir.join("user-file.txt"), b"keep me").unwrap();

    let options = NewHtmlProjectOptions {
        raw_path: Some(project_dir.to_string_lossy().to_string()),
        force: true,
    };
    let mut prompt = ScriptedPrompt::new(vec![
        String::from("1"),
        String::from(""),
        String::from("y"),
        String::from("n"),
    ]);

    let result = create_html_project_template_with_prompt(options, &mut prompt);
    assert!(result.is_ok(), "expected success, got: {result:?}");

    let config_content = fs::read_to_string(project_dir.join("config.moth")).unwrap();
    assert!(config_content.contains("name = \""));

    let user_content = fs::read_to_string(project_dir.join("user-file.txt")).unwrap();
    assert_eq!(user_content, "keep me");
}

#[test]
fn creates_full_default_scaffold_and_reports_every_path() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let target = named_target(project_dir.clone(), "My_Project", false);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();

    // Every scaffold-owned path exists on disk.
    assert!(project_dir.join("config.moth").exists());
    assert!(project_dir.join("@page.moth").exists());
    assert!(!project_dir.join("src").exists());
    assert!(!project_dir.join("_dev").exists());
    assert!(!project_dir.join("_release").exists());

    assert!(report.created.contains(&PathBuf::from("config.moth")));
    assert!(report.created.contains(&PathBuf::from("@page.moth")));
    assert!(!report.created.contains(&PathBuf::from("src")));
    assert!(report.created.contains(&PathBuf::from(".gitignore")));
    assert!(report.replaced.is_empty());
    assert!(report.updated.is_empty());
}

#[test]
fn generated_files_exactly_match_templates() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let target = named_target(project_dir.clone(), "Test_Site", false);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    write_scaffold(&target, false, &mut prompt).unwrap();

    let config = fs::read_to_string(project_dir.join("config.moth")).unwrap();
    assert!(config.contains("name = \"Test_Site\""));
    assert!(!config.contains("version"));
    assert!(!config.contains("entry_root"));
    assert_eq!(
        config,
        start_page_scaffolding::config_template("Test_Site").unwrap()
    );
    assert_eq!(
        fs::read_to_string(project_dir.join("@page.moth")).unwrap(),
        start_page_scaffolding::page_template()
    );
}

#[test]
fn gitignore_is_created_when_absent_and_confirmed_or_skipped_when_declined() {
    // Absent .gitignore plus confirmation creates the exact default content.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();
    assert_eq!(
        fs::read_to_string(project_dir.join(".gitignore")).unwrap(),
        start_page_scaffolding::gitignore_template()
    );
    assert!(report.created.contains(&PathBuf::from(".gitignore")));

    // Absent .gitignore plus decline skips it and writes nothing.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();
    assert!(!project_dir.join(".gitignore").exists());
    assert!(report.skipped.contains(&PathBuf::from(".gitignore")));
}

#[test]
fn gitignore_appends_when_present_without_moth_output_block_or_skips_when_declined() {
    // Existing .gitignore without the Moth output rules: confirmation appends the block.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), "node_modules/\n").unwrap();
    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();
    let content = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert!(content.contains("node_modules/"));
    assert!(content.contains("# Moth generated output"));
    assert!(content.lines().any(|line| line.trim() == "/_dev"));
    assert!(content.lines().any(|line| line.trim() == "/_release"));
    assert!(report.updated.contains(&PathBuf::from(".gitignore")));

    // Existing .gitignore without the Moth output rules: decline leaves it unchanged.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), "node_modules/\n").unwrap();
    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();
    assert_eq!(
        fs::read_to_string(project_dir.join(".gitignore")).unwrap(),
        "node_modules/\n"
    );
    assert!(report.skipped.contains(&PathBuf::from(".gitignore")));
}

#[test]
fn gitignore_is_updated_when_only_the_old_dev_rule_is_present() {
    // The old `/dev` rule does not satisfy the pair of current output rules.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), "/dev\n").unwrap();
    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();
    let content = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert!(content.lines().any(|line| line.trim() == "/dev"));
    assert!(content.lines().any(|line| line.trim() == "/_dev"));
    assert!(content.lines().any(|line| line.trim() == "/_release"));
    assert!(report.updated.contains(&PathBuf::from(".gitignore")));
}

#[test]
fn gitignore_appends_only_missing_rules() {
    // One existing rule must not be duplicated when the other rule is appended.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), "/_dev\n").unwrap();
    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    write_scaffold(&target, false, &mut prompt).unwrap();
    let content = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert_eq!(
        content
            .lines()
            .filter(|line| line.trim() == "/_dev")
            .count(),
        1
    );
    assert_eq!(
        content
            .lines()
            .filter(|line| line.trim() == "/_release")
            .count(),
        1
    );
    assert_eq!(
        content
            .lines()
            .filter(|line| line.trim() == "# Moth generated output")
            .count(),
        1
    );
}

#[test]
fn gitignore_is_skipped_when_moth_output_block_already_present() {
    // Both rules must be present; exact, trailing-slash, and whitespace-padded forms count.
    for existing in [
        "/_dev\n/_release\n",
        "/_dev/\n/_release/\n",
        "  /_dev  \n  /_release  \n",
        "noise\n/_dev\nmore\n/_release\n",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        fs::write(project_dir.join(".gitignore"), existing).unwrap();
        let target = named_target(project_dir.clone(), "Test_Site", true);
        let mut prompt = ScriptedPrompt::new(Vec::new());

        let report = write_scaffold(&target, false, &mut prompt).unwrap();

        assert_eq!(
            fs::read_to_string(project_dir.join(".gitignore")).unwrap(),
            existing
        );
        assert!(report.skipped.contains(&PathBuf::from(".gitignore")));
    }
}

#[test]
fn invalid_direct_scaffold_name_is_rejected_without_writing_files() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let target = named_target(project_dir.clone(), "My Site", false);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    let error = write_scaffold(&target, false, &mut prompt).unwrap_err();

    assert!(error.contains("Invalid project name `My Site`"));
    assert!(
        !project_dir.exists(),
        "validation should precede file writes"
    );
}

#[test]
fn write_scaffold_directly_refuses_regular_replacement_without_force() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();

    let target = named_target(project_dir.clone(), "Test_Site", false);
    let mut prompt = ScriptedPrompt::new(Vec::new());
    run_preflight_checks(&target, false, &mut prompt).unwrap();

    // The entry appears after preflight, so the writer must enforce the force contract itself.
    fs::write(project_dir.join("config.moth"), b"original").unwrap();

    let error = write_scaffold(&target, false, &mut prompt).unwrap_err();
    assert!(error.contains("config.moth") && error.contains("already exists"));
    assert!(error.contains("--force"));
    assert_eq!(
        fs::read(project_dir.join("config.moth")).unwrap(),
        b"original"
    );
    assert!(
        !project_dir.join("@page.moth").exists(),
        "the direct writer must stop before writing the later scaffold file"
    );
}

#[test]
fn force_replaces_scaffold_owned_files_only() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::create_dir(project_dir.join("_dev")).unwrap();
    fs::create_dir(project_dir.join("_release")).unwrap();
    fs::write(project_dir.join("config.moth"), b"old config").unwrap();
    fs::write(project_dir.join("@page.moth"), b"old page").unwrap();
    fs::write(project_dir.join("_dev/user-output.js"), b"old dev output").unwrap();
    fs::write(
        project_dir.join("_release/user-output.js"),
        b"old release output",
    )
    .unwrap();
    fs::write(project_dir.join("user-file.txt"), b"keep me").unwrap();

    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("y")]);

    let report = write_scaffold(&target, true, &mut prompt).unwrap();

    assert!(report.replaced.contains(&PathBuf::from("config.moth")));
    assert!(report.replaced.contains(&PathBuf::from("@page.moth")));
    assert_eq!(
        fs::read_to_string(project_dir.join("config.moth")).unwrap(),
        start_page_scaffolding::config_template("Test_Site").unwrap()
    );
    assert_eq!(
        fs::read_to_string(project_dir.join("@page.moth")).unwrap(),
        start_page_scaffolding::page_template()
    );
    assert_eq!(
        fs::read(project_dir.join("_dev/user-output.js")).unwrap(),
        b"old dev output"
    );
    assert_eq!(
        fs::read(project_dir.join("_release/user-output.js")).unwrap(),
        b"old release output"
    );

    let user_content = fs::read_to_string(project_dir.join("user-file.txt")).unwrap();
    assert_eq!(user_content, "keep me");
}

#[test]
fn force_replacement_breaks_hard_links_without_touching_external_files() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();

    let outside_config = temp.path().join("outside-config.moth");
    let outside_page = temp.path().join("outside-page.moth");
    fs::write(&outside_config, b"outside config").unwrap();
    fs::write(&outside_page, b"outside page").unwrap();
    fs::hard_link(&outside_config, project_dir.join("config.moth")).unwrap();
    fs::hard_link(&outside_page, project_dir.join("@page.moth")).unwrap();

    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);

    let report = write_scaffold(&target, true, &mut prompt).unwrap();

    assert!(report.replaced.contains(&PathBuf::from("config.moth")));
    assert!(report.replaced.contains(&PathBuf::from("@page.moth")));
    assert_eq!(
        fs::read_to_string(project_dir.join("config.moth")).unwrap(),
        start_page_scaffolding::config_template("Test_Site").unwrap()
    );
    assert_eq!(
        fs::read_to_string(project_dir.join("@page.moth")).unwrap(),
        start_page_scaffolding::page_template()
    );
    assert_eq!(
        fs::read(&outside_config).unwrap(),
        b"outside config",
        "replacement must not write through the old hard-linked inode"
    );
    assert_eq!(
        fs::read(&outside_page).unwrap(),
        b"outside page",
        "replacement must not write through the old hard-linked inode"
    );
}

#[cfg(unix)]
#[test]
fn atomic_replacement_preserves_ordinary_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join("config.moth"), b"old config").unwrap();
    fs::write(project_dir.join(".gitignore"), b"custom\n").unwrap();
    fs::set_permissions(
        project_dir.join("config.moth"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    fs::set_permissions(
        project_dir.join(".gitignore"),
        fs::Permissions::from_mode(0o604),
    )
    .unwrap();

    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);
    write_scaffold(&target, true, &mut prompt).unwrap();

    for (relative, expected_mode) in [("config.moth", 0o640), (".gitignore", 0o604)] {
        let mode = fs::metadata(project_dir.join(relative))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, expected_mode,
            "{relative} should retain its ordinary permission bits"
        );
    }
}

#[cfg(unix)]
#[test]
fn failed_staging_leaves_the_existing_file_and_no_temporary_file() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join("config.moth"), b"original").unwrap();
    fs::set_permissions(&project_dir, fs::Permissions::from_mode(0o555)).unwrap();

    // This covers temporary-file creation failure; the separately injected publication failure
    // covers cleanup after a complete staged write. Root can create files in a read-only
    // directory, so skip when the failure cannot be staged.
    if tempfile::NamedTempFile::new_in(&project_dir).is_ok() {
        fs::set_permissions(&project_dir, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(Vec::new());
    let result = write_scaffold(&target, true, &mut prompt);
    fs::set_permissions(&project_dir, fs::Permissions::from_mode(0o755)).unwrap();

    let error = result.unwrap_err();
    assert!(
        error.contains("config.moth") && error.contains("staging"),
        "{error}"
    );
    assert_eq!(
        fs::read(project_dir.join("config.moth")).unwrap(),
        b"original",
        "a staging failure must not truncate the original file"
    );
    let entries = fs::read_dir(&project_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("config.moth")],
        "a failed staging attempt must not leave a temporary file"
    );
}

#[test]
fn failed_publication_after_staging_leaves_the_original_and_no_temporary_file() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let config = project_dir.join("config.moth");
    fs::write(&config, b"original").unwrap();

    let error = replace_existing_regular_file_with_failed_publish_for_test(
        &config,
        "replacement",
        "config.moth",
    )
    .unwrap_err();

    assert!(
        error.contains("config.moth") && error.contains("publishing"),
        "{error}"
    );
    assert_eq!(
        fs::read(&config).unwrap(),
        b"original",
        "a publication failure must leave the original file intact"
    );
    let entries = fs::read_dir(&project_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("config.moth")],
        "the staged temporary file must be removed after publish failure"
    );
}

#[test]
fn gitignore_is_never_overwritten_even_with_force() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), b"custom\n").unwrap();

    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);

    let report = write_scaffold(&target, true, &mut prompt).unwrap();

    let content = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert_eq!(content, "custom\n");
    assert!(report.skipped.contains(&PathBuf::from(".gitignore")));
}

#[test]
fn gitignore_append_breaks_hard_links_without_touching_the_external_file() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let outside = temp.path().join("outside.gitignore");
    fs::write(&outside, b"node_modules/\n").unwrap();
    fs::hard_link(&outside, project_dir.join(".gitignore")).unwrap();

    let target = named_target(project_dir.clone(), "Test_Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();

    let updated = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert!(updated.contains("node_modules/"));
    assert!(updated.contains("# Moth generated output"));
    assert!(updated.lines().any(|line| line.trim() == "/_dev"));
    assert!(updated.lines().any(|line| line.trim() == "/_release"));
    assert!(report.updated.contains(&PathBuf::from(".gitignore")));
    assert_eq!(
        fs::read(&outside).unwrap(),
        b"node_modules/\n",
        "the append must not write through the old hard-linked inode"
    );
}

#[test]
fn gitignore_disappearance_after_append_confirmation_is_not_recreated() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let gitignore = project_dir.join(".gitignore");
    fs::write(&gitignore, b"custom\n").unwrap();

    let removed = gitignore.clone();
    let mut prompt = MutatingConfirmPrompt::new(move || fs::remove_file(&removed).unwrap());
    let target = named_target(project_dir, "Test_Site", true);

    let error = write_scaffold(&target, false, &mut prompt).unwrap_err();

    assert!(
        error.contains(".gitignore") && error.contains("disappeared"),
        "{error}"
    );
    assert!(
        !gitignore.exists(),
        "a disappeared .gitignore must not be silently recreated"
    );
    assert_eq!(prompt.messages.len(), 1);
}

#[cfg(unix)]
#[test]
fn gitignore_symlink_substitution_after_confirmation_does_not_write_the_target() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let gitignore = project_dir.join(".gitignore");
    fs::write(&gitignore, b"custom\n").unwrap();
    let outside = temp.path().join("outside.gitignore");
    fs::write(&outside, b"outside\n").unwrap();

    let replaced = gitignore.clone();
    let target_path = outside.clone();
    let mut prompt = MutatingConfirmPrompt::new(move || {
        fs::remove_file(&replaced).unwrap();
        symlink(&target_path, &replaced).unwrap();
    });
    let target = named_target(project_dir, "Test_Site", true);

    let error = write_scaffold(&target, false, &mut prompt).unwrap_err();

    assert!(
        error.contains(".gitignore") && error.contains("symbolic link"),
        "{error}"
    );
    assert_eq!(
        fs::read(&outside).unwrap(),
        b"outside\n",
        "the late symlink target must not be written"
    );
    assert!(
        fs::symlink_metadata(&gitignore)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the substituted symlink must remain in place after refusal"
    );
}

#[test]
fn gitignore_content_changed_after_confirmation_is_appended_from_the_latest_text() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let gitignore = project_dir.join(".gitignore");
    fs::write(&gitignore, b"custom\n").unwrap();

    let changed = gitignore.clone();
    let mut prompt = MutatingConfirmPrompt::new(move || {
        fs::write(&changed, b"custom\nlate-rule\n").unwrap();
    });
    let target = named_target(project_dir.clone(), "Test_Site", true);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();

    let updated = fs::read_to_string(&gitignore).unwrap();
    assert!(
        updated.contains("late-rule"),
        "the append must use the post-confirmation snapshot: {updated:?}"
    );
    assert!(updated.contains("# Moth generated output"));
    assert!(updated.lines().any(|line| line.trim() == "/_dev"));
    assert!(updated.lines().any(|line| line.trim() == "/_release"));
    assert!(report.updated.contains(&PathBuf::from(".gitignore")));
}

#[test]
fn gitignore_completed_after_confirmation_is_skipped_without_rewriting() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let gitignore = project_dir.join(".gitignore");
    fs::write(&gitignore, b"custom\n").unwrap();

    let completed = gitignore.clone();
    let mut prompt = MutatingConfirmPrompt::new(move || {
        fs::write(&completed, b"/_dev\n/_release\n").unwrap();
    });
    let target = named_target(project_dir, "Test_Site", true);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();

    assert_eq!(fs::read(&gitignore).unwrap(), b"/_dev\n/_release\n");
    assert!(report.skipped.contains(&PathBuf::from(".gitignore")));
    assert!(report.updated.is_empty());
}

#[test]
fn write_failure_returns_precise_error() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let target = named_target(project_dir.clone(), "Test_Site", false);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    // Create a file where the project directory should be to force create_dir_all to fail.
    fs::write(&project_dir, b"").unwrap();

    let error = write_scaffold(&target, false, &mut prompt).unwrap_err();
    assert!(error.contains("Project creation failed"));
}

#[test]
fn existing_contains_moth_output_block_recognizes_exact_rules_and_rejects_near_matches() {
    // Both rules, with trailing slashes, surrounding whitespace, or noise, are present.
    for present in [
        "/_dev\n/_release\n",
        "/_dev/\n/_release/\n",
        "  /_dev  \n  /_release  \n",
        "noise\n/_dev\nmore\n/_release\n",
    ] {
        assert!(existing_contains_moth_output_block(present), "{present:?}");
    }

    // Missing one rule, the old /dev rule, and near matches must not count as present.
    for absent in [
        "/_dev\n",
        "/_release\n",
        "/dev\n",
        "/dev\n/_release\n",
        "/_dev\n/dev\n",
        "/_device\n/_release\n",
        "prefix/_dev\n/_release\n",
        "/_dev/**\n/_release\n",
        "# /_dev\n/_release\n",
        "/_dev/foo\n/_release\n",
        "_dev\n/_release\n",
    ] {
        assert!(!existing_contains_moth_output_block(absent), "{absent:?}");
    }
}
