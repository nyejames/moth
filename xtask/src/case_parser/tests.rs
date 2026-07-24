use super::*;
use std::fs;
use std::path::PathBuf;

fn write_case_file(name: &str, contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "moth_{}_{}_cases.txt",
        name,
        std::process::id()
    ));

    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn test_parse_line_simple() {
    let line = "check speed-test.moth";
    let case = parse_line(line).unwrap();
    assert_eq!(case.group_name, "ungrouped");
    assert_eq!(case.command, "check");
    assert_eq!(case.args, vec!["speed-test.moth"]);
    assert_eq!(case.name, "check_speed-test_bst");
}

#[test]
fn test_parse_line_multiple_args() {
    let line = "build docs src output";
    let case = parse_line(line).unwrap();
    assert_eq!(case.command, "build");
    assert_eq!(case.args, vec!["docs", "src", "output"]);
    assert_eq!(case.name, "build_docs_src_output");
}

#[test]
fn test_parse_line_multiple_spaces() {
    let line = "check   speed-test.moth";
    let case = parse_line(line).unwrap();
    assert_eq!(case.command, "check");
    assert_eq!(case.args, vec!["speed-test.moth"]);
}

#[test]
fn test_parse_line_quoted_arg() {
    let line = r#"check "path with spaces.moth""#;
    let case = parse_line(line).unwrap();
    assert_eq!(case.command, "check");
    assert_eq!(case.args, vec!["path with spaces.moth"]);
}

#[test]
fn test_parse_line_empty() {
    let line = "";
    let result = parse_line(line);
    assert!(result.is_err());
}

#[test]
fn test_parse_line_unclosed_quote() {
    let line = r#"check "unclosed"#;
    let result = parse_line(line);
    assert!(result.is_err());
}

#[test]
fn test_sanitize_case_name() {
    assert_eq!(
        sanitize_case_name("check", &["speed-test.moth".to_string()]),
        "check_speed-test_bst"
    );
    assert_eq!(
        sanitize_case_name("check", &["path/to/file.moth".to_string()]),
        "check_path_to_file_bst"
    );
}

#[test]
fn test_tokenize_line() {
    let tokens = tokenize_line("check file.moth").unwrap();
    assert_eq!(tokens, vec!["check", "file.moth"]);

    let tokens = tokenize_line("check   file.moth").unwrap();
    assert_eq!(tokens, vec!["check", "file.moth"]);

    let tokens = tokenize_line(r#"check "file name.moth""#).unwrap();
    assert_eq!(tokens, vec!["check", "file name.moth"]);
}

#[test]
fn test_parse_cases_defaults_to_ungrouped() {
    let path = write_case_file("default_group", "check benchmarks/speed-test.moth\n");
    let cases = parse_cases(&path).unwrap();

    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].group_name, "ungrouped");

    fs::remove_file(path).unwrap();
}

#[test]
fn test_parse_cases_applies_group_directive_to_following_cases() {
    let path = write_case_file(
        "single_group",
        "# group: core\ncheck benchmarks/speed-test.moth\nbuild benchmarks/speed-test.moth\n",
    );
    let cases = parse_cases(&path).unwrap();

    assert_eq!(cases.len(), 2);
    assert!(cases.iter().all(|case| case.group_name == "core"));

    fs::remove_file(path).unwrap();
}

#[test]
fn test_parse_cases_handles_multiple_group_directives() {
    let path = write_case_file(
        "multiple_groups",
        "\
# group: core
check benchmarks/speed-test.moth

# group: docs
check docs

# group: stress
check benchmarks/template-stress.moth
",
    );
    let cases = parse_cases(&path).unwrap();

    assert_eq!(cases.len(), 3);
    assert_eq!(cases[0].group_name, "core");
    assert_eq!(cases[1].group_name, "docs");
    assert_eq!(cases[2].group_name, "stress");

    fs::remove_file(path).unwrap();
}

#[test]
fn test_parse_cases_ignores_normal_comments() {
    let path = write_case_file(
        "normal_comments",
        "\
# Benchmark cases
# this is just a comment
check docs
",
    );
    let cases = parse_cases(&path).unwrap();

    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].group_name, "ungrouped");
    assert_eq!(cases[0].command, "check");

    fs::remove_file(path).unwrap();
}

#[test]
fn test_parse_cases_rejects_empty_group_directive() {
    let path = write_case_file("empty_group", "# group:   \ncheck docs\n");
    let error = parse_cases(&path).unwrap_err();

    assert!(error.contains("group directive requires a non-empty name"));

    fs::remove_file(path).unwrap();
}

#[test]
fn test_parse_cases_preserves_quoted_paths_with_group() {
    let path = write_case_file("quoted_path", "# group: docs\ncheck \"docs with spaces\"\n");
    let cases = parse_cases(&path).unwrap();

    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].group_name, "docs");
    assert_eq!(cases[0].args, vec!["docs with spaces"]);
    assert_eq!(cases[0].name, "check_docs_with_spaces");

    fs::remove_file(path).unwrap();
}
