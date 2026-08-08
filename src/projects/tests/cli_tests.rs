//! Tests for CLI command parsing and validation.

#[cfg(feature = "timers")]
use super::run_build_command_with_output_plan;
use super::{
    Command, build_warnings_messages, compact_whitespace, get_command, help_build_flag_entries,
    integration_run_status, is_standalone_version_request, run_build_command,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, FileKind, OutputFile, Project};
use crate::build_system::output::{BuilderKind, CleanupPolicy, OutputOwner};
use crate::compiler_frontend::Flag;
#[cfg(feature = "timers")]
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, RuleDiagnosticKind,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_tests::integration_test_runner::{
    BackendId, IntegrationRunSummary, TestRunnerOptions,
};
use crate::compiler_tests::test_support::temp_dir;
use crate::projects::command_status::CommandStatus;
use crate::projects::dev_server::DevServerOptions;
use crate::projects::html_project::new_html_project::NewHtmlProjectOptions;
use crate::projects::settings::Config;
#[cfg(feature = "timers")]
use crate::timing::start_benchmark_collection;
use std::fs;
use std::path::PathBuf;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn dev_command_uses_default_options() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
fn build_command_writes_the_validated_directory_output_plan() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("cli_directory_output_plan");
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\ndev_folder #= \"preview\"\noutput_folder #= \"release\"\n",
    )
    .expect("should write project config");
    fs::write(
        source_root.join("@page.moth"),
        "#[:<h1>CLI Directory Plan</h1>]\n",
    )
    .expect("should write page source");

    let status = run_build_command(
        root.to_str()
            .expect("temporary project path should be valid UTF-8"),
        &[],
    );
    assert_eq!(status, CommandStatus::Success);
    assert!(root.join("preview/index.html").exists());
    assert!(!root.join("dev/index.html").exists());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

/// An output-plan failure still reaches the command's single timing finish.
#[cfg(feature = "timers")]
#[test]
fn failed_output_plan_records_the_build_command_total() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let root = temp_dir("cli_failed_output_plan_timer");
    fs::create_dir_all(&root).expect("should create temporary project root");
    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    // The outer raw session intentionally makes the inner command session a
    // rejected no-op. Stage guards still record into the outer session, which
    // lets this unit test inspect the command total without rendering it.
    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let status = run_build_command_with_output_plan(
        entry_file
            .to_str()
            .expect("temporary path should be valid UTF-8"),
        &[],
        |_| Err(CompilerError::compiler_error("forced output-plan failure")),
    );
    let snapshot = timing_session.finish();

    assert_eq!(status, CommandStatus::Failure);
    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "command.build.total")
            .count(),
        1,
        "the failed output-plan path must finish the command total before the session drains"
    );
    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "build.output.total")
            .count(),
        1,
        "the failed output-plan path must finish the output segment before the session drains"
    );

    fs::remove_dir_all(&root).expect("should remove temporary project root");
}

#[test]
fn build_command_supports_mixed_path_and_flag_ordering() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error =
        get_command(&args(&["build", "--wat"])).expect_err("unknown build flag should fail");
    assert!(error.contains("Unknown build flag"));
    assert!(error.contains("--release"));
    assert!(error.contains("--html-wasm"));
}

#[test]
fn new_html_command_uses_current_directory_when_path_is_missing() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["dev", "main.moth", "--port", "invalid"]))
        .expect_err("invalid port should fail");
    assert!(error.contains("Invalid --port value"));
}

#[test]
fn dev_command_rejects_unknown_flags() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error =
        get_command(&args(&["dev", "main.moth", "--wat"])).expect_err("unknown flag should fail");
    assert!(error.contains("Unknown dev flag"));
}

#[test]
fn dev_command_rejects_missing_flag_values() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let host_error =
        get_command(&args(&["dev", "main.moth", "--host"])).expect_err("missing host value");
    assert!(host_error.contains("Missing value for --host"));

    let port_error =
        get_command(&args(&["dev", "main.moth", "--port"])).expect_err("missing port value");
    assert!(port_error.contains("Missing value for --port"));
}

#[test]
fn dev_command_rejects_zero_poll_interval() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["dev", "main.moth", "--poll-interval-ms", "0"]))
        .expect_err("zero interval should fail");
    assert!(error.contains("greater than zero"));
}

#[test]
fn dev_command_supports_path_and_flag_ordering() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["new", "html", "a", "b"]))
        .expect_err("multiple new html paths should fail");
    assert!(error.contains("at most one path"));
}

#[test]
fn new_html_command_parses_force_flag_after_path() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error =
        get_command(&args(&["new", "html", "--yes"])).expect_err("unknown flag should fail");
    assert!(error.contains("Unknown new flag"));
}

#[test]
fn build_command_rejects_force_flag() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["build", "--force"])).expect_err("build --force should fail");
    assert!(error.contains("Unknown build flag"));
}

#[test]
fn tests_command_uses_default_options() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let command = get_command(&args(&["tests"])).expect("tests command should parse");
    assert_eq!(
        command,
        Command::CompilerTests {
            options: TestRunnerOptions {
                show_warnings: true,
                terse: false,
                ..TestRunnerOptions::default()
            },
        }
    );
}

#[test]
fn tests_command_parses_backend_filter() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["tests", "--audit", "--audit"]))
        .expect_err("duplicate audit should fail");
    assert!(error.contains("--audit") && error.contains("at most once"));
}

#[test]
fn tests_command_rejects_duplicate_tag_values() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["tests", "--tag", "borrows", "--tag", "borrows"]))
        .expect_err("duplicate tag should fail");
    assert!(
        error.contains("duplicate --tag"),
        "unexpected error: {error}"
    );
}

#[test]
fn tests_command_rejects_missing_selection_values() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for option in ["--case", "--tag", "--contract", "--backend"] {
        let error = get_command(&args(&["tests", option]))
            .expect_err("missing selection value should fail");
        assert!(error.contains("Missing value"), "unexpected error: {error}");
    }
}

#[test]
fn tests_command_rejects_unknown_backend_and_positional_arguments() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let backend_error = get_command(&args(&["tests", "--backend", "wasm"]))
        .expect_err("unsupported backend should fail");
    assert!(backend_error.contains("Invalid value for --backend"));
    assert!(backend_error.contains("Unsupported backend"));

    let positional_error = get_command(&args(&["tests", "case_id"]))
        .expect_err("positional test argument should fail");
    assert!(positional_error.contains("does not accept positional arguments"));
}

#[test]
fn compact_whitespace_collapses_multiline_to_one_line() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    assert_eq!(compact_whitespace("hello\nworld"), "hello world");
    assert_eq!(
        compact_whitespace("line1\n\n  line2  \nline3"),
        "line1 line2 line3"
    );
    assert_eq!(compact_whitespace(""), "");
    assert_eq!(compact_whitespace("  "), "");
    assert_eq!(compact_whitespace("single"), "single");
}

#[test]
fn tests_command_rejects_unknown_flags() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error =
        get_command(&args(&["tests", "--wat"])).expect_err("unknown tests flag should fail");
    assert!(error.contains("Unknown tests flag"));
    assert!(error.contains("--terse"));
}

#[test]
fn check_command_uses_default_options() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["check", "a.moth", "b.moth"]))
        .expect_err("multiple check paths should fail");
    assert!(error.contains("at most one path"));
}

#[test]
fn build_command_returns_exact_flags() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    assert!(is_standalone_version_request(&args(&["--version"])));
    assert!(is_standalone_version_request(&args(&["-v"])));
    assert!(is_standalone_version_request(&args(&["-V"])));
}

#[test]
fn standalone_version_request_rejects_non_version_flags() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let entries = help_build_flag_entries();
    let joined = entries.join("\n");

    assert!(joined.contains("--release"));
    assert!(joined.contains("--html-wasm"));
    assert!(!joined.contains("--hide-warnings"));
    assert!(!joined.contains("--hide-timers"));
    assert!(!joined.contains("--show-warnings"));
}

#[test]
fn integration_run_status_reflects_suite_correctness() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let correct = IntegrationRunSummary {
        total_tests: 5,
        passed_tests: 3,
        failed_tests: 0,
        expected_failures: 2,
        unexpected_successes: 0,
    };
    assert_eq!(integration_run_status(correct), CommandStatus::Success);

    let incorrect = IntegrationRunSummary {
        total_tests: 5,
        passed_tests: 2,
        failed_tests: 1,
        expected_failures: 1,
        unexpected_successes: 1,
    };
    assert_eq!(integration_run_status(incorrect), CommandStatus::Failure);
}

#[test]
fn tests_command_terse_flag_sets_terse_true() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let command = get_command(&args(&["tests", "--terse"])).expect("tests --terse should parse");
    assert_eq!(
        command,
        Command::CompilerTests {
            options: TestRunnerOptions {
                show_warnings: true,
                terse: true,
                ..TestRunnerOptions::default()
            },
        }
    );
}

#[test]
fn tests_command_terse_composes_with_filters() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for args_slice in [
        vec!["tests", "--terse", "--case", "case_a"],
        vec!["tests", "--case", "case_a", "--terse"],
        vec!["tests", "--terse", "--tag", "integration"],
        vec!["tests", "--terse", "--contract", "lang.case"],
        vec!["tests", "--terse", "--backend", "html"],
    ] {
        assert!(
            get_command(&args(&args_slice)).is_ok(),
            "terse should compose with filters: {:?}",
            args_slice
        );
    }
}

#[test]
fn tests_command_rejects_duplicate_terse() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["tests", "--terse", "--terse"]))
        .expect_err("duplicate --terse should fail");
    assert!(error.contains("--terse") && error.contains("at most once"));
}

#[test]
fn tests_command_rejects_terse_list_in_either_order() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for values in [
        vec!["tests", "--terse", "--list"],
        vec!["tests", "--list", "--terse"],
    ] {
        let error = get_command(&args(&values)).expect_err("terse+list should fail");
        assert!(
            error.contains("--terse") && error.contains("--list"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn tests_command_rejects_terse_audit_in_either_order() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for values in [
        vec!["tests", "--terse", "--audit"],
        vec!["tests", "--audit", "--terse"],
    ] {
        let error = get_command(&args(&values)).expect_err("terse+audit should fail");
        assert!(
            error.contains("--terse") && error.contains("--audit"),
            "unexpected error: {error}"
        );
    }
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
        output_owner: OutputOwner {
            builder: BuilderKind::Html,
            profile: BuildProfile::Dev,
        },
        directory_output_plan: None,
    }
}

#[test]
fn successful_build_without_warnings_has_no_warning_messages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let build_result = build_result_with_warnings(Vec::new());

    assert!(
        build_warnings_messages(&build_result).is_none(),
        "a successful build with no warnings should not produce a CompilerMessages container"
    );
}

#[test]
fn successful_build_with_warnings_exposes_warning_messages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
