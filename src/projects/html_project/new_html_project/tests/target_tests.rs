//! Tests for target path resolution and interactive project placement.
//!
//! Each test owns one algorithm or one interactive branch of
//! `resolve_project_target`: lexical path normalization, tilde-form routing,
//! the Windows home fallback chain, and the omitted/dot/relative/absolute,
//! existing-directory, missing-directory, project-name, and non-empty-detection
//! branches. Same-family cases share one labelled owner instead of repeating
//! near-identical fixtures.

use crate::projects::html_project::new_html_project::prompt_tests::ScriptedPrompt;
use crate::projects::html_project::new_html_project::target::{
    HomeEnv, expand_tilde, normalize_path, resolve_project_target,
};
use std::fs;
use std::path::PathBuf;

use std::collections::HashMap;

/// In-process mock environment for platform-independent home-resolution tests.
///
/// WHAT: Holds a fixed set of environment-variable values so tests can exercise
/// the `HOME`, `USERPROFILE` and `HOMEDRIVE`/`HOMEPATH` fallback chain without
/// mutating the process-global environment.
/// WHY: Direct `std::env::set_var` races under parallel test execution and
/// cannot simulate Windows variables on a Unix host.
struct MockHomeEnv {
    values: HashMap<String, String>,
    windows: bool,
}

impl MockHomeEnv {
    fn empty() -> Self {
        Self {
            values: HashMap::new(),
            windows: false,
        }
    }

    fn with(key: &str, value: &str) -> Self {
        let mut values = HashMap::new();
        values.insert(key.to_owned(), value.to_owned());
        Self {
            values,
            windows: false,
        }
    }

    fn and(mut self, key: &str, value: &str) -> Self {
        self.values.insert(key.to_owned(), value.to_owned());
        self
    }

    fn windows(mut self) -> Self {
        self.windows = true;
        self
    }
}

impl HomeEnv for MockHomeEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }

    fn is_windows(&self) -> bool {
        self.windows
    }
}

#[test]
fn normalize_path_collapses_components_and_contains_root() {
    // dot and dot-dot components collapse.
    assert_eq!(
        normalize_path(&PathBuf::from("/a/b/../c")),
        PathBuf::from("/a/c")
    );
    assert_eq!(
        normalize_path(&PathBuf::from("./site")),
        PathBuf::from("site")
    );
    assert_eq!(
        normalize_path(&PathBuf::from("a/./b")),
        PathBuf::from("a/b")
    );

    // parent-dot at the root cannot escape the root.
    assert_eq!(
        normalize_path(&PathBuf::from("/a/../../b")),
        PathBuf::from("/b")
    );
}

#[test]
fn expand_tilde_routes_tilde_forms() {
    let env = MockHomeEnv::with("HOME", "/mock/home");

    // bare `~` and slash-separated remainders expand to home.
    assert_eq!(
        expand_tilde("~", &env).unwrap(),
        PathBuf::from("/mock/home")
    );
    assert_eq!(
        expand_tilde("~/site", &env).unwrap(),
        PathBuf::from("/mock/home/site")
    );
    assert_eq!(
        expand_tilde("~/nested/deep", &env).unwrap(),
        PathBuf::from("/mock/home/nested/deep")
    );

    // backslash-separated remainders expand and normalise to forward slashes.
    assert_eq!(
        expand_tilde("~\\site", &env).unwrap(),
        PathBuf::from("/mock/home/site")
    );
    assert_eq!(
        expand_tilde("~\\nested\\deep", &env).unwrap(),
        PathBuf::from("/mock/home/nested/deep")
    );

    // named-user shorthand is left unchanged so it is not misread as home.
    assert_eq!(
        expand_tilde("~other", &env).unwrap(),
        PathBuf::from("~other")
    );
    assert_eq!(
        expand_tilde("~other/site", &env).unwrap(),
        PathBuf::from("~other/site")
    );
    assert_eq!(
        expand_tilde("~other\\site", &env).unwrap(),
        PathBuf::from("~other\\site")
    );

    // non-tilde paths pass through unchanged.
    assert_eq!(
        expand_tilde("/absolute/path", &env).unwrap(),
        PathBuf::from("/absolute/path")
    );
    assert_eq!(
        expand_tilde("relative/path", &env).unwrap(),
        PathBuf::from("relative/path")
    );
}

#[test]
fn resolve_home_uses_windows_fallbacks_and_rejects_incomplete_pairs() {
    // Windows: `USERPROFILE` is used when `HOME` is absent.
    let userprofile = MockHomeEnv::with("USERPROFILE", "C:\\Users\\test").windows();
    assert_eq!(
        expand_tilde("~/site", &userprofile).unwrap(),
        PathBuf::from("C:\\Users\\test/site")
    );

    // Windows: `HOMEDRIVE` plus `HOMEPATH` is used when `HOME` and
    // `USERPROFILE` are absent.
    let drive_and_path = MockHomeEnv::with("HOMEDRIVE", "C:")
        .and("HOMEPATH", "\\Users\\test")
        .windows();
    assert_eq!(
        expand_tilde("~/site", &drive_and_path).unwrap(),
        PathBuf::from("C:\\Users\\test/site")
    );

    // Windows: an incomplete `HOMEDRIVE`/`HOMEPATH` pair is rejected.
    let drive_only = MockHomeEnv::with("HOMEDRIVE", "C:").windows();
    assert_eq!(
        expand_tilde("~/site", &drive_only).unwrap_err(),
        "Could not determine home directory for '~' expansion."
    );
    let path_only = MockHomeEnv::with("HOMEPATH", "\\Users\\test").windows();
    assert_eq!(
        expand_tilde("~/site", &path_only).unwrap_err(),
        "Could not determine home directory for '~' expansion."
    );

    // Empty `HOME` is treated as unset and falls through to `USERPROFILE`.
    let empty_home = MockHomeEnv::with("HOME", "")
        .and("USERPROFILE", "C:\\Users\\test")
        .windows();
    assert_eq!(
        expand_tilde("~/site", &empty_home).unwrap(),
        PathBuf::from("C:\\Users\\test/site")
    );

    // Non-Windows hosts ignore Windows-only home variables.
    let non_windows = MockHomeEnv::with("USERPROFILE", "C:\\Users\\test")
        .and("HOMEDRIVE", "C:")
        .and("HOMEPATH", "\\Users\\test");
    assert_eq!(
        expand_tilde("~/site", &non_windows).unwrap_err(),
        "Could not determine home directory for '~' expansion."
    );

    // No home variable available at all is rejected.
    let absent = MockHomeEnv::empty();
    assert_eq!(
        expand_tilde("~/site", &absent).unwrap_err(),
        "Could not determine home directory for '~' expansion."
    );
}

#[test]
fn omitted_path_confirms_or_cancels() {
    // Confirming creates in the current directory and reports the prompt.
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().to_path_buf();
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("")]);

    let resolved = resolve_project_target(None, &current, &mut prompt).unwrap();

    assert_eq!(resolved.project_dir, current);
    assert!(!resolved.target_was_non_empty);
    assert!(prompt.messages[0].contains("No project path specified"));
    assert!(prompt.messages[0].contains(&current.to_string_lossy().to_string()));

    // Declining cancels project creation.
    let current = PathBuf::from("/tmp");
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);

    let error = resolve_project_target(None, &current, &mut prompt).unwrap_err();

    assert_eq!(error, "Cancelled project creation.");
}

#[test]
fn dot_resolves_to_current_directory() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().to_path_buf();
    let mut prompt = ScriptedPrompt::new(vec![String::from("")]);

    let resolved = resolve_project_target(Some(String::from(".")), &current, &mut prompt).unwrap();

    assert_eq!(resolved.project_dir, current);
    assert_eq!(
        resolved.project_name,
        super::target::package_identifier_from_display_name(
            current.file_name().unwrap().to_str().unwrap()
        )
    );
}

#[test]
fn relative_child_path_resolves_under_current_directory() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().to_path_buf();
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("")]);

    let resolved =
        resolve_project_target(Some(String::from("site")), &current, &mut prompt).unwrap();

    assert_eq!(resolved.project_dir, current.join("site"));
}

#[test]
fn absolute_path_is_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("absolute-site");
    let current = PathBuf::from("/tmp");
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("")]);

    let resolved = resolve_project_target(
        Some(target.to_string_lossy().to_string()),
        &current,
        &mut prompt,
    )
    .unwrap();

    assert_eq!(resolved.project_dir, target);
}

#[test]
fn existing_directory_menu_offers_use_child_or_cancel() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().join("existing");
    fs::create_dir(&existing).unwrap();
    let current = temp.path().to_path_buf();

    // Option 1 creates the project inside the existing directory.
    let mut prompt = ScriptedPrompt::new(vec![String::from("1"), String::from("")]);
    let resolved =
        resolve_project_target(Some(String::from("existing")), &current, &mut prompt).unwrap();
    assert_eq!(resolved.project_dir, existing);

    // Option 2 creates a named child folder inside the existing directory.
    let mut prompt = ScriptedPrompt::new(vec![
        String::from("2"),
        String::from("child"),
        String::from(""),
    ]);
    let resolved =
        resolve_project_target(Some(String::from("existing")), &current, &mut prompt).unwrap();
    assert_eq!(resolved.project_dir, existing.join("child"));

    // Option 3 cancels project creation.
    let mut prompt = ScriptedPrompt::new(vec![String::from("3")]);
    let error =
        resolve_project_target(Some(String::from("existing")), &current, &mut prompt).unwrap_err();
    assert_eq!(error, "Cancelled project creation.");
}

#[test]
fn missing_directory_confirms_or_cancels() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().to_path_buf();

    // Confirming creates the missing nested directories.
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("")]);
    let resolved =
        resolve_project_target(Some(String::from("new/nested/site")), &current, &mut prompt)
            .unwrap();
    assert_eq!(resolved.project_dir, current.join("new/nested/site"));
    assert!(prompt.messages[0].contains("directories that do not exist"));

    // Declining cancels project creation.
    let mut prompt = ScriptedPrompt::new(vec![String::from("n")]);
    let error =
        resolve_project_target(Some(String::from("new/site")), &current, &mut prompt).unwrap_err();
    assert_eq!(error, "Cancelled project creation.");
}

#[test]
fn project_name_defaults_overrides_or_trims() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().to_path_buf();

    // An empty name defaults to the directory basename, sanitised to a package identifier.
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("")]);
    let resolved =
        resolve_project_target(Some(String::from("my-site")), &current, &mut prompt).unwrap();
    assert_eq!(resolved.project_name, "my_site");

    // An explicit name overrides the basename and is sanitised the same way.
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("My Custom Site")]);
    let resolved =
        resolve_project_target(Some(String::from("my-site")), &current, &mut prompt).unwrap();
    assert_eq!(resolved.project_name, "My_Custom_Site");

    // A padded name is trimmed, then sanitised.
    let mut prompt = ScriptedPrompt::new(vec![String::from("y"), String::from("  Padded Name  ")]);
    let resolved =
        resolve_project_target(Some(String::from("dir")), &current, &mut prompt).unwrap();
    assert_eq!(resolved.project_name, "Padded_Name");
}

#[test]
fn detects_non_empty_existing_directory() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().join("non-empty");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("file.txt"), b"content").unwrap();
    let current = temp.path().to_path_buf();
    let mut prompt = ScriptedPrompt::new(vec![String::from("1"), String::from("")]);

    let resolved =
        resolve_project_target(Some(String::from("non-empty")), &current, &mut prompt).unwrap();

    assert!(resolved.target_was_non_empty);
}
