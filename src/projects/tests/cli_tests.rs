//! Tests for CLI command parsing and validation.

use super::{
    Command, build_warnings_messages, get_command, help_build_flag_entries,
    integration_tests_exit_code, is_standalone_version_request,
};
use crate::build_system::build::{BuildResult, CleanupPolicy, FileKind, OutputFile, Project};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, RuleDiagnosticKind,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_tests::integration_test_runner::{
    BackendId, IntegrationRunSummary, TestRunnerOptions,
};
use crate::projects::dev_server::DevServerOptions;
use crate::projects::html_project::new_html_project::NewHtmlProjectOptions;
use crate::projects::settings::Config;
use std::path::PathBuf;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn dev_command_uses_default_options() {
    let command = get_command(&args(&["dev", "main.moth"])).expect("command should parse");
    assert_eq!(
        command,
        Command::Dev {
            path: String::from("main.moth"),
            options: DevServerOptions::default(),
            flags: Vec::new(),
        }
    );
}

#[test]
fn build_command_uses_current_directory_when_path_is_missing() {
    let command = get_command(&args(&["build"])).expect("build command should parse");
    assert_eq!(
        command,
        Command::Build {
            path: String::new(),
            flags: Vec::new(),
        }
    );
}

#[test]
fn build_command_supports_mixed_path_and_flag_ordering() {
    let command =
        get_command(&args(&["build", "--release", "main.moth"])).expect("command should parse");
    assert_eq!(
        command,
        Command::Build {
            path: String::from("main.moth"),
            flags: vec![Flag::Release],
        }
    );
}

#[test]
fn build_command_rejects_unknown_flags() {
    let error =
        get_command(&args(&["build", "--wat"])).expect_err("unknown build flag should fail");
    assert!(error.contains("Unknown build flag"));
    assert!(error.contains("--release"));
    assert!(error.contains("--html-wasm"));
}

#[test]
fn new_html_command_uses_current_directory_when_path_is_missing() {
    let command = get_command(&args(&["new", "html"])).expect("new html command should parse");
    assert_eq!(
        command,
        Command::NewHTMLProject(NewHtmlProjectOptions {
            raw_path: None,
            force: false,
        })
    );
}

#[test]
fn new_html_command_parses_project_path() {
    let command = get_command(&args(&["new", "html", "site"])).expect("new html should parse");
    assert_eq!(
        command,
        Command::NewHTMLProject(NewHtmlProjectOptions {
            raw_path: Some(String::from("site")),
            force: false,
        })
    );
}

#[test]
fn dev_command_parses_custom_host_port_and_poll_interval() {
    let command = get_command(&args(&[
        "dev",
        "main.moth",
        "--host",
        "0.0.0.0",
        "--port",
        "7777",
        "--poll-interval-ms",
        "120",
    ]))
    .expect("command should parse");

    assert_eq!(
        command,
        Command::Dev {
            path: String::from("main.moth"),
            options: DevServerOptions {
                host: String::from("0.0.0.0"),
                port: 7777,
                poll_interval_ms: 120,
            },
            flags: Vec::new(),
        }
    );
}

#[test]
fn dev_command_rejects_invalid_port_values() {
    let error = get_command(&args(&["dev", "main.moth", "--port", "invalid"]))
        .expect_err("invalid port should fail");
    assert!(error.contains("Invalid --port value"));
}

#[test]
fn dev_command_rejects_unknown_flags() {
    let error =
        get_command(&args(&["dev", "main.moth", "--wat"])).expect_err("unknown flag should fail");
    assert!(error.contains("Unknown dev flag"));
}

#[test]
fn dev_command_rejects_missing_flag_values() {
    let host_error =
        get_command(&args(&["dev", "main.moth", "--host"])).expect_err("missing host value");
    assert!(host_error.contains("Missing value for --host"));

    let port_error =
        get_command(&args(&["dev", "main.moth", "--port"])).expect_err("missing port value");
    assert!(port_error.contains("Missing value for --port"));
}

#[test]
fn dev_command_rejects_zero_poll_interval() {
    let error = get_command(&args(&["dev", "main.moth", "--poll-interval-ms", "0"]))
        .expect_err("zero interval should fail");
    assert!(error.contains("greater than zero"));
}

#[test]
fn dev_command_supports_path_and_flag_ordering() {
    let command = get_command(&args(&[
        "dev",
        "--host",
        "localhost",
        "main.moth",
        "--poll-interval-ms",
        "900",
    ]))
    .expect("command should parse with mixed ordering");

    assert_eq!(
        command,
        Command::Dev {
            path: String::from("main.moth"),
            options: DevServerOptions {
                host: String::from("localhost"),
                port: 6342,
                poll_interval_ms: 900,
            },
            flags: Vec::new(),
        }
    );
}

#[test]
fn new_html_command_rejects_multiple_paths() {
    let error = get_command(&args(&["new", "html", "a", "b"]))
        .expect_err("multiple new html paths should fail");
    assert!(error.contains("at most one path"));
}

#[test]
fn new_html_command_parses_force_flag_after_path() {
    let command =
        get_command(&args(&["new", "html", "site", "--force"])).expect("command should parse");
    assert_eq!(
        command,
        Command::NewHTMLProject(NewHtmlProjectOptions {
            raw_path: Some(String::from("site")),
            force: true,
        })
    );
}

#[test]
fn new_html_command_parses_force_flag_before_path() {
    let command =
        get_command(&args(&["new", "html", "--force", "site"])).expect("command should parse");
    assert_eq!(
        command,
        Command::NewHTMLProject(NewHtmlProjectOptions {
            raw_path: Some(String::from("site")),
            force: true,
        })
    );
}

#[test]
fn new_html_command_rejects_unknown_flags() {
    let error =
        get_command(&args(&["new", "html", "--yes"])).expect_err("unknown flag should fail");
    assert!(error.contains("Unknown new flag"));
}

#[test]
fn build_command_rejects_force_flag() {
    let error = get_command(&args(&["build", "--force"])).expect_err("build --force should fail");
    assert!(error.contains("Unknown build flag"));
}

#[test]
fn tests_command_uses_default_backend_selection() {
    let command = get_command(&args(&["tests"])).expect("tests command should parse");
    assert_eq!(
        command,
        Command::CompilerTests {
            options: TestRunnerOptions {
                show_warnings: true,
                ..TestRunnerOptions::default()
            },
        }
    );
}

#[test]
fn tests_command_parses_backend_filter() {
    let command = get_command(&args(&["tests", "--backend", "html_wasm"]))
        .expect("tests backend filter should parse");
    assert_eq!(
        command,
        Command::CompilerTests {
            options: TestRunnerOptions {
                show_warnings: true,
                backend_filter: Some(BackendId::HtmlWasm),
                ..TestRunnerOptions::default()
            },
        }
    );
}

#[test]
fn tests_command_parses_audit_mode() {
    let command = get_command(&args(&["tests", "--audit"])).expect("audit mode should parse");
    assert_eq!(
        command,
        Command::CompilerTests {
            options: TestRunnerOptions {
                show_warnings: true,
                audit: true,
                ..TestRunnerOptions::default()
            },
        }
    );
}

#[test]
fn tests_command_parses_composable_selection_options() {
    let command = get_command(&args(&[
        "tests",
        "--tag",
        "integration",
        "--case",
        "arithmetic_operator_precedence",
        "--tag",
        "language",
        "--contract",
        "language.operator_precedence",
        "--backend",
        "html",
        "--list",
    ]))
    .expect("tests selection options should parse");

    assert_eq!(
        command,
        Command::CompilerTests {
            options: TestRunnerOptions {
                show_warnings: true,
                case_id: Some(String::from("arithmetic_operator_precedence")),
                tag_filters: vec![String::from("integration"), String::from("language")],
                contract: Some(String::from("language.operator_precedence")),
                backend_filter: Some(BackendId::Html),
                list: true,
                ..TestRunnerOptions::default()
            },
        }
    );
}

#[test]
fn tests_command_rejects_duplicate_singleton_options() {
    for duplicate in [
        vec!["--case", "one", "--case", "two"],
        vec!["--contract", "one", "--contract", "two"],
        vec!["--backend", "html", "--backend", "html_wasm"],
        vec!["--list", "--list"],
    ] {
        let mut values = vec!["tests"];
        values.extend(duplicate);
        let error = get_command(&args(&values)).expect_err("duplicate option should fail");
        assert!(
            error.contains("at most one") || error.contains("at most once"),
            "{error}"
        );
    }
}

#[test]
fn tests_command_rejects_audit_filters_in_any_argument_order() {
    for values in [
        vec!["tests", "--audit", "--case", "case"],
        vec!["tests", "--case", "case", "--audit"],
        vec!["tests", "--audit", "--tag", "language"],
        vec!["tests", "--contract", "language.case", "--audit"],
        vec!["tests", "--audit", "--backend", "html"],
        vec!["tests", "--list", "--audit"],
    ] {
        let error = get_command(&args(&values)).expect_err("audit filter should fail");
        assert!(
            error.contains("--audit") && error.contains("cannot be combined"),
            "{error}"
        );
    }
}

#[test]
fn tests_command_rejects_duplicate_audit() {
    let error = get_command(&args(&["tests", "--audit", "--audit"]))
        .expect_err("duplicate audit should fail");
    assert!(error.contains("--audit") && error.contains("at most once"));
}

#[test]
fn tests_command_rejects_duplicate_tag_values() {
    let error = get_command(&args(&["tests", "--tag", "borrows", "--tag", "borrows"]))
        .expect_err("duplicate tag should fail");
    assert!(
        error.contains("duplicate --tag"),
        "unexpected error: {error}"
    );
}

#[test]
fn tests_command_rejects_missing_selection_values() {
    for option in ["--case", "--tag", "--contract", "--backend"] {
        let error = get_command(&args(&["tests", option]))
            .expect_err("missing selection value should fail");
        assert!(error.contains("Missing value"), "unexpected error: {error}");
    }
}

#[test]
fn tests_command_rejects_unknown_backend_and_positional_arguments() {
    let backend_error = get_command(&args(&["tests", "--backend", "wasm"]))
        .expect_err("unsupported backend should fail");
    assert!(backend_error.contains("Invalid value for --backend"));
    assert!(backend_error.contains("Unsupported backend"));

    let positional_error = get_command(&args(&["tests", "case_id"]))
        .expect_err("positional test argument should fail");
    assert!(positional_error.contains("does not accept positional arguments"));
}

#[test]
fn tests_command_rejects_unknown_flags() {
    let error =
        get_command(&args(&["tests", "--wat"])).expect_err("unknown tests flag should fail");
    assert!(error.contains("Unknown tests flag"));
}

#[test]
fn check_command_uses_default_options() {
    let command = get_command(&args(&["check"])).expect("check command should parse");
    assert_eq!(
        command,
        Command::Check {
            path: String::new(),
            terse: false,
        }
    );
}

#[test]
fn check_command_parses_path_and_terse_flag() {
    let command = get_command(&args(&["check", "main.moth", "--terse"]))
        .expect("check command should parse path and terse flag");
    assert_eq!(
        command,
        Command::Check {
            path: String::from("main.moth"),
            terse: true,
        }
    );
}

#[test]
fn check_command_supports_mixed_argument_ordering() {
    let command = get_command(&args(&["check", "--terse", "main.moth"]))
        .expect("check command should parse mixed argument ordering");
    assert_eq!(
        command,
        Command::Check {
            path: String::from("main.moth"),
            terse: true,
        }
    );
}

#[test]
fn check_command_rejects_multiple_paths() {
    let error = get_command(&args(&["check", "a.moth", "b.moth"]))
        .expect_err("multiple check paths should fail");
    assert!(error.contains("at most one path"));
}

#[test]
fn build_command_returns_exact_flags() {
    let release = get_command(&args(&["build", "--release"])).expect("release flag should parse");
    assert_eq!(
        release,
        Command::Build {
            path: String::new(),
            flags: vec![Flag::Release],
        }
    );

    let wasm = get_command(&args(&["build", "--html-wasm"])).expect("html-wasm flag should parse");
    assert_eq!(
        wasm,
        Command::Build {
            path: String::new(),
            flags: vec![Flag::HtmlWasm],
        }
    );

    let both = get_command(&args(&["build", "--release", "--html-wasm"]))
        .expect("both flags should parse");
    assert_eq!(
        both,
        Command::Build {
            path: String::new(),
            flags: vec![Flag::Release, Flag::HtmlWasm],
        }
    );
}

#[test]
fn dev_command_returns_exact_flags() {
    let release = get_command(&args(&["dev", "--release"])).expect("release flag should parse");
    assert_eq!(
        release,
        Command::Dev {
            path: String::new(),
            options: DevServerOptions::default(),
            flags: vec![Flag::Release],
        }
    );

    let wasm = get_command(&args(&["dev", "--html-wasm"])).expect("html-wasm flag should parse");
    assert_eq!(
        wasm,
        Command::Dev {
            path: String::new(),
            options: DevServerOptions::default(),
            flags: vec![Flag::HtmlWasm],
        }
    );
}

#[test]
fn build_command_rejects_removed_warning_flags() {
    for removed in &["--hide-warnings", "--hide-timers", "--show-warnings"] {
        let error = get_command(&args(&["build", removed]))
            .expect_err("removed flag should be rejected by build");
        assert!(
            error.contains("Unknown build flag"),
            "build should reject {removed} as unknown"
        );
    }
}

#[test]
fn dev_command_rejects_removed_warning_flags() {
    for removed in &["--hide-warnings", "--hide-timers", "--show-warnings"] {
        let error = get_command(&args(&["dev", "main.moth", removed]))
            .expect_err("removed flag should be rejected by dev");
        assert!(
            error.contains("Unknown dev flag"),
            "dev should reject {removed} as unknown"
        );
    }
}

#[test]
fn new_command_rejects_removed_warning_flags() {
    for removed in &["--hide-warnings", "--hide-timers", "--show-warnings"] {
        let error = get_command(&args(&["new", "html", removed]))
            .expect_err("removed flag should be rejected by new");
        assert!(
            error.contains("Unknown new flag"),
            "new should reject {removed} as unknown"
        );
    }
}

#[test]
fn new_html_command_rejects_build_flags() {
    for flag in &["--release", "--html-wasm"] {
        let error = get_command(&args(&["new", "html", flag]))
            .expect_err("build flag should be rejected by new");
        assert!(
            error.contains("Unknown new flag"),
            "new should reject {flag}"
        );
    }
}

#[test]
fn check_command_rejects_build_flags() {
    for flag in &["--release", "--html-wasm"] {
        let error = get_command(&args(&["check", flag]))
            .expect_err("build flag should be rejected by check");
        assert!(
            error.contains("Unknown check flag"),
            "check should reject {flag}"
        );
    }
}

#[test]
fn tests_command_rejects_build_flags() {
    for flag in &["--release", "--html-wasm"] {
        let error = get_command(&args(&["tests", flag]))
            .expect_err("build flag should be rejected by tests");
        assert!(
            error.contains("Unknown tests flag"),
            "tests should reject {flag}"
        );
    }
}

#[test]
fn standalone_version_request_recognises_all_spellings() {
    assert!(is_standalone_version_request(&args(&["--version"])));
    assert!(is_standalone_version_request(&args(&["-v"])));
    assert!(is_standalone_version_request(&args(&["-V"])));
}

#[test]
fn standalone_version_request_rejects_non_version_flags() {
    assert!(!is_standalone_version_request(&args(&["--release"])));
    for removed in &["--hide-warnings", "--hide-timers", "--show-warnings"] {
        assert!(!is_standalone_version_request(&args(&[removed])));
    }
    assert!(!is_standalone_version_request(&args(&[
        "--version",
        "--release"
    ])));
    assert!(!is_standalone_version_request(&args(&["build"])));
    assert!(!is_standalone_version_request(&[]));
}

#[test]
fn help_advertises_accepted_flags_but_not_removed_spelling() {
    let entries = help_build_flag_entries();
    let joined = entries.join("\n");

    assert!(joined.contains("--release"));
    assert!(joined.contains("--html-wasm"));
    assert!(!joined.contains("--hide-warnings"));
    assert!(!joined.contains("--hide-timers"));
    assert!(!joined.contains("--show-warnings"));
}

#[test]
fn integration_tests_exit_code_reflects_suite_correctness() {
    let correct = IntegrationRunSummary {
        total_tests: 5,
        passed_tests: 3,
        failed_tests: 0,
        expected_failures: 2,
        unexpected_successes: 0,
    };
    assert_eq!(integration_tests_exit_code(correct), 0);

    let incorrect = IntegrationRunSummary {
        total_tests: 5,
        passed_tests: 2,
        failed_tests: 1,
        expected_failures: 1,
        unexpected_successes: 1,
    };
    assert_eq!(integration_tests_exit_code(incorrect), 1);
}

fn build_result_with_warnings(warnings: Vec<CompilerDiagnostic>) -> BuildResult {
    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings,
        string_table: StringTable::new(),
    }
}

#[test]
fn successful_build_without_warnings_has_no_warning_messages() {
    let build_result = build_result_with_warnings(Vec::new());

    assert!(
        build_warnings_messages(&build_result).is_none(),
        "a successful build with no warnings should not produce a CompilerMessages container"
    );
}

#[test]
fn successful_build_with_warnings_exposes_warning_messages() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("unused_value");
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnusedVariable),
        DiagnosticSeverity::Warning,
        SourceLocation::default(),
        DiagnosticPayload::UnusedName { name },
    );

    let build_result = build_result_with_warnings(vec![warning]);
    let messages = build_warnings_messages(&build_result).expect("warnings should be wrapped");

    assert_eq!(messages.warning_count(), 1);
    assert_eq!(messages.error_count(), 0);
    assert!(
        !messages.has_errors(),
        "successful-build warnings must not be treated as errors"
    );
}
