//! Tests for the `moth new html` file-writing scaffold.
//!
//! Each test owns one distinct scaffold contract. Same-family decision pairs
//! and inventories share one labelled owner so failure localization stays clear,
//! while the direct `write_scaffold` IO policy is kept separate from the
//! end-to-end `create_html_project_template_with_prompt` orchestration that
//! protects the no-write and force command boundaries.

use crate::projects::html_project::new_html_project::prompt_tests::ScriptedPrompt;
use crate::projects::html_project::new_html_project::scaffold::{
    SCAFFOLD_DIRECTORIES, existing_contains_dev_block, find_scaffold_conflicts,
    run_preflight_checks, write_scaffold,
};
use crate::projects::html_project::new_html_project::start_page_scaffolding;
use crate::projects::html_project::new_html_project::target::ResolvedProjectTarget;
use crate::projects::html_project::new_html_project::{
    NewHtmlProjectOptions, create_html_project_template_with_prompt,
};
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

#[test]
fn find_scaffold_conflicts_reports_exact_owned_set_and_excludes_directories_and_gitignore() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().to_path_buf();

    // Scaffold-owned files are the exact conflict set, in declared order.
    for file in ["config.moth", "src/@page.moth"] {
        if let Some(parent) = project_dir.join(file).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(project_dir.join(file), b"old").unwrap();
    }
    assert_eq!(
        find_scaffold_conflicts(&project_dir),
        vec!["config.moth", "src/@page.moth",]
    );

    // Scaffold-owned directories and a user .gitignore are never conflicts.
    let clean = tempfile::tempdir().unwrap();
    let clean_dir = clean.path().to_path_buf();
    for dir in SCAFFOLD_DIRECTORIES {
        fs::create_dir_all(clean_dir.join(dir)).unwrap();
    }
    fs::write(clean_dir.join(".gitignore"), b"old").unwrap();
    assert!(find_scaffold_conflicts(&clean_dir).is_empty());
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
    assert!(config_content.contains("name #= "));

    let user_content = fs::read_to_string(project_dir.join("user-file.txt")).unwrap();
    assert_eq!(user_content, "keep me");
}

#[test]
fn creates_full_default_scaffold_and_reports_every_path() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let target = named_target(project_dir.clone(), "My Project", false);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();

    // Every scaffold-owned path exists on disk.
    assert!(project_dir.join("config.moth").exists());
    assert!(project_dir.join("src/@page.moth").exists());
    assert!(project_dir.join("lib").exists());
    assert!(!project_dir.join("dev/.moth_manifest").exists());
    assert!(!project_dir.join("release/.moth_manifest").exists());
    assert!(project_dir.join(".gitignore").exists());

    // The default scaffold creates every path and replaces, updates, or skips nothing.
    assert!(report.created.contains(&PathBuf::from("config.moth")));
    assert!(report.created.contains(&PathBuf::from("src/@page.moth")));
    assert!(report.created.contains(&PathBuf::from("lib")));
    assert!(report.created.contains(&PathBuf::from(".gitignore")));
    assert!(report.replaced.is_empty());
    assert!(report.updated.is_empty());
    assert!(report.skipped.is_empty());

    // The lib directory is created empty.
    let mut entries = fs::read_dir(project_dir.join("lib")).unwrap();
    assert!(entries.next().is_none());
}

#[test]
fn generated_files_exactly_match_templates() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let target = named_target(project_dir.clone(), "Test Site", false);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    write_scaffold(&target, false, &mut prompt).unwrap();

    assert_eq!(
        fs::read_to_string(project_dir.join("config.moth")).unwrap(),
        start_page_scaffolding::config_template("Test Site")
    );
    assert_eq!(
        fs::read_to_string(project_dir.join("src/@page.moth")).unwrap(),
        start_page_scaffolding::page_template()
    );
}

#[test]
fn gitignore_is_created_when_absent_and_confirmed_or_skipped_when_declined() {
    // Absent .gitignore plus confirmation creates the exact default content.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    let target = named_target(project_dir.clone(), "Test Site", true);
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
    let target = named_target(project_dir.clone(), "Test Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();
    assert!(!project_dir.join(".gitignore").exists());
    assert!(report.skipped.contains(&PathBuf::from(".gitignore")));
}

#[test]
fn gitignore_appends_when_present_without_dev_block_or_skips_when_declined() {
    // Existing .gitignore without a /dev block: confirmation appends the block.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), "node_modules/\n").unwrap();
    let target = named_target(project_dir.clone(), "Test Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();
    let content = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert!(content.contains("node_modules/"));
    assert!(content.contains("# Moth"));
    assert!(content.contains("/dev"));
    assert!(report.updated.contains(&PathBuf::from(".gitignore")));

    // Existing .gitignore without a /dev block: decline leaves it unchanged.
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), "node_modules/\n").unwrap();
    let target = named_target(project_dir.clone(), "Test Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);

    let report = write_scaffold(&target, false, &mut prompt).unwrap();
    assert_eq!(
        fs::read_to_string(project_dir.join(".gitignore")).unwrap(),
        "node_modules/\n"
    );
    assert!(report.skipped.contains(&PathBuf::from(".gitignore")));
}

#[test]
fn gitignore_is_skipped_when_dev_block_already_present() {
    // Both exact /dev and trailing-slash /dev/ forms are recognised as present.
    for existing in ["/dev\n", "/dev/\n"] {
        let temp = tempfile::tempdir().unwrap();
        let project_dir = temp.path().join("project");
        fs::create_dir(&project_dir).unwrap();
        fs::write(project_dir.join(".gitignore"), existing).unwrap();
        let target = named_target(project_dir.clone(), "Test Site", true);
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
fn project_name_is_escaped_in_config() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let target = named_target(project_dir.clone(), r#"Say "hello"\back"#, false);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    write_scaffold(&target, false, &mut prompt).unwrap();

    let content = fs::read_to_string(project_dir.join("config.moth")).unwrap();
    assert!(content.contains(r#"name #= "Say \"hello\"\\back""#));
}

#[test]
fn force_replaces_scaffold_owned_files_only() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::create_dir(project_dir.join("src")).unwrap();
    fs::create_dir(project_dir.join("dev")).unwrap();
    fs::create_dir(project_dir.join("release")).unwrap();
    fs::write(project_dir.join("config.moth"), b"old config").unwrap();
    fs::write(project_dir.join("src/@page.moth"), b"old page").unwrap();
    fs::write(project_dir.join("dev/user-output.js"), b"old dev output").unwrap();
    fs::write(
        project_dir.join("release/user-output.js"),
        b"old release output",
    )
    .unwrap();
    fs::write(project_dir.join("user-file.txt"), b"keep me").unwrap();

    let target = named_target(project_dir.clone(), "Test Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("y")]);

    let report = write_scaffold(&target, true, &mut prompt).unwrap();

    assert!(report.replaced.contains(&PathBuf::from("config.moth")));
    assert!(report.replaced.contains(&PathBuf::from("src/@page.moth")));
    assert_eq!(
        fs::read(project_dir.join("dev/user-output.js")).unwrap(),
        b"old dev output"
    );
    assert_eq!(
        fs::read(project_dir.join("release/user-output.js")).unwrap(),
        b"old release output"
    );

    let user_content = fs::read_to_string(project_dir.join("user-file.txt")).unwrap();
    assert_eq!(user_content, "keep me");
}

#[test]
fn gitignore_is_never_overwritten_even_with_force() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join(".gitignore"), b"custom\n").unwrap();

    let target = named_target(project_dir.clone(), "Test Site", true);
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);

    let report = write_scaffold(&target, true, &mut prompt).unwrap();

    let content = fs::read_to_string(project_dir.join(".gitignore")).unwrap();
    assert_eq!(content, "custom\n");
    assert!(report.skipped.contains(&PathBuf::from(".gitignore")));
}

#[test]
fn write_failure_returns_precise_error() {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("project");
    let target = named_target(project_dir.clone(), "Test Site", false);
    let mut prompt = ScriptedPrompt::new(vec![String::from("y")]);

    // Create a file where the project directory should be to force create_dir_all to fail.
    fs::write(&project_dir, b"").unwrap();

    let error = write_scaffold(&target, false, &mut prompt).unwrap_err();
    assert!(error.contains("Project creation failed"));
}

#[test]
fn existing_contains_dev_block_recognizes_exact_rules_and_rejects_near_matches() {
    // Exact /dev and /dev/ lines, with surrounding whitespace or noise, are present.
    for present in ["/dev\n", "/dev/\n", "  /dev  \n", "noise\n/dev\nmore\n"] {
        assert!(existing_contains_dev_block(present), "{present:?}");
    }

    // Near matches that must not be treated as the Moth /dev rule.
    for absent in [
        "/device\n",
        "prefix/dev\n",
        "/dev/**\n",
        "# /dev\n",
        "/dev/foo\n",
        "dev\n",
    ] {
        assert!(!existing_contains_dev_block(absent), "{absent:?}");
    }
}
