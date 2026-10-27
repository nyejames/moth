//! Tests for CLI parsing, command status and terminal presentation.

#[cfg(unix)]
use super::convert_cli_args;
use super::{
    Command, CommandInputArgument, CommandStatus, command_input_set, compact_whitespace,
    format_terse_summary_line, get_command, help_build_flag_entries, integration_run_status,
    is_standalone_version_request,
};
use crate::scaffold::NewHtmlProjectOptions;
use moth::projects::dev_server::DevServerOptions;
use moth::projects::integration_tests::{BackendId, IntegrationRunSummary, TestRunnerOptions};
#[cfg(feature = "boracle")]
use moth::projects::{
    BoracleDump, BoracleExperiment, BoracleReferenceRuleSet, BoracleRuleSelection,
};
use moth::projects::{
    BuildCommandLocation, BuildConfigInputSet, BuildConfigInputValue, BuildConfigValueLocation,
    BuildInputName, Flag, PrimitiveBuildValue,
};
#[cfg(feature = "boracle")]
use std::collections::BTreeSet;
use std::time::Duration;

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
            inputs: Vec::new(),
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
            inputs: Vec::new(),
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
            inputs: Vec::new(),
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

    let Command::Dev {
        path,
        options,
        flags,
        inputs,
    } = command
    else {
        panic!("dev command expected");
    };
    assert_eq!(path, "main.moth");
    assert_eq!(options.host, "0.0.0.0");
    assert_eq!(options.port, 7777);
    assert_eq!(options.poll_interval_ms, 120);
    assert!(flags.is_empty());
    assert!(inputs.is_empty());
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

    let Command::Dev {
        path,
        options,
        flags,
        inputs,
    } = command
    else {
        panic!("dev command expected");
    };
    assert_eq!(path, "main.moth");
    assert_eq!(options.host, "localhost");
    assert_eq!(options.port, 6342);
    assert_eq!(options.poll_interval_ms, 900);
    assert!(flags.is_empty());
    assert!(inputs.is_empty());
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
fn tests_command_uses_default_options() {
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
fn compact_whitespace_collapses_multiline_to_one_line() {
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
    let error =
        get_command(&args(&["tests", "--wat"])).expect_err("unknown tests flag should fail");
    assert!(error.contains("Unknown tests flag"));
    assert!(error.contains("--terse"));
}

#[test]
fn check_command_uses_default_options() {
    let command = get_command(&args(&["check"])).expect("check command should parse");
    assert_eq!(
        command,
        Command::Check {
            path: String::new(),
            terse: false,
            inputs: Vec::new(),
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
            inputs: Vec::new(),
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
            inputs: Vec::new(),
        }
    );
}

#[test]
fn check_command_rejects_multiple_paths() {
    let error = get_command(&args(&["check", "a.moth", "b.moth"]))
        .expect_err("multiple check paths should fail");
    assert!(error.contains("at most one path"));
}
#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_parses_typed_dump_and_repeated_experiments() {
    let command = get_command(&args(&[
        "boracle",
        "main.moth",
        "--dump",
        "witnesses",
        "--experiment",
        "dead-exclusive-loan",
        "--experiment",
        "dead-exclusive-loan",
    ]))
    .expect("boracle command should parse");
    assert_eq!(
        command,
        Command::Boracle {
            path: String::from("main.moth"),
            dump: BoracleDump::Witnesses,
            rule_selection: BoracleRuleSelection {
                reference_rule_set: BoracleReferenceRuleSet::V1,
                experiments: BTreeSet::from([BoracleExperiment::DeadExclusiveLoan]),
            },
        }
    );
}

#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_defaults_to_empty_experiment_set() {
    let command =
        get_command(&args(&["boracle", "main.moth"])).expect("boracle command should parse");
    assert_eq!(
        command,
        Command::Boracle {
            path: String::from("main.moth"),
            dump: BoracleDump::Problem,
            rule_selection: BoracleRuleSelection::default(),
        }
    );
}

#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_requires_source_path() {
    let error =
        get_command(&args(&["boracle"])).expect_err("Boracle command should require a source path");
    assert!(error.contains("requires one source path"));
}

#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_rejects_unknown_experiment() {
    let error = get_command(&args(&["boracle", "main.moth", "--experiment", "unknown"]))
        .expect_err("unknown Boracle experiment should fail");
    assert!(error.contains("Unknown Boracle experiment"));
}

#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_rejects_comma_separated_experiments() {
    let error = get_command(&args(&[
        "boracle",
        "main.moth",
        "--experiment",
        "dead-exclusive-loan,dead-exclusive-loan",
    ]))
    .expect_err("comma-separated Boracle experiments should fail");
    assert!(error.contains("Unknown Boracle experiment"));
}

#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_rejects_unknown_dump() {
    let error = get_command(&args(&["boracle", "main.moth", "--dump", "unknown"]))
        .expect_err("unknown Boracle dump should fail");
    assert!(error.contains("Invalid value for --dump"));
}

#[test]
fn build_command_returns_exact_flags() {
    let release = get_command(&args(&["build", "--release"])).expect("release flag should parse");
    assert_eq!(
        release,
        Command::Build {
            path: String::new(),
            flags: vec![Flag::Release],
            inputs: Vec::new(),
        }
    );

    let wasm = get_command(&args(&["build", "--html-wasm"])).expect("html-wasm flag should parse");
    assert_eq!(
        wasm,
        Command::Build {
            path: String::new(),
            flags: vec![Flag::HtmlWasm],
            inputs: Vec::new(),
        }
    );

    let both = get_command(&args(&["build", "--release", "--html-wasm"]))
        .expect("both flags should parse");
    assert_eq!(
        both,
        Command::Build {
            path: String::new(),
            flags: vec![Flag::Release, Flag::HtmlWasm],
            inputs: Vec::new(),
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
            inputs: Vec::new(),
        }
    );

    let wasm = get_command(&args(&["dev", "--html-wasm"])).expect("html-wasm flag should parse");
    assert_eq!(
        wasm,
        Command::Dev {
            path: String::new(),
            options: DevServerOptions::default(),
            flags: vec![Flag::HtmlWasm],
            inputs: Vec::new(),
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

    if cfg!(feature = "boracle") || cfg!(feature = "boracle_campaign") {
        assert!(
            get_command(&args(&["boracle", "main.moth"])).is_ok(),
            "the Boracle command should be available whenever a Boracle feature is enabled"
        );
    }
}

#[test]
fn terse_summary_line_matches_clean_success_contract() {
    let summary = format_terse_summary_line(Duration::from_millis(5), 0, 0);
    assert_eq!(summary, "Done in 5ms. No errors or warnings.");
}

#[test]
fn integration_run_status_reflects_suite_correctness() {
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
    let error = get_command(&args(&["tests", "--terse", "--terse"]))
        .expect_err("duplicate --terse should fail");
    assert!(error.contains("--terse") && error.contains("at most once"));
}

#[test]
fn tests_command_rejects_terse_list_in_either_order() {
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

// -------------------------
//  Shared `--input name=value` command parser
// -------------------------

fn command_inputs(command: &Command) -> &[CommandInputArgument] {
    match command {
        Command::Build { inputs, .. }
        | Command::Check { inputs, .. }
        | Command::Dev { inputs, .. } => inputs,
        _ => panic!("command variant does not carry build-config inputs"),
    }
}

/// Classify one parsed command's retained `--input` arguments into the shared input set.
fn input_set(command: &Command) -> BuildConfigInputSet {
    command_input_set(command_inputs(command)).expect("retained command inputs should classify")
}

/// Look up the value a command retained under `name`, cloned out of its input set.
fn retained_value(command: &Command, name: &str) -> BuildConfigInputValue {
    let name = BuildInputName::new(name).expect("test input name should validate");
    let inputs = input_set(command);
    inputs
        .get(&name)
        .unwrap_or_else(|| panic!("command should retain {name:?}"))
        .value()
        .clone()
}

fn single_input(argument: &str) -> (BuildInputName, BuildConfigInputValue) {
    let (name_text, value_text) = argument.split_once('=').expect("test argument has '='");
    (
        BuildInputName::new(name_text).expect("test input name should validate"),
        BuildConfigInputValue::from_command_text(value_text)
            .expect("test input value should classify"),
    )
}

/// Every command variant shares one parser and one classification path: the same argument text
/// classifies to the same typed value with the same command location on build, check and dev,
/// and no project config, source contract or build-system state exists in this test.
#[test]
fn every_command_classifies_typed_values_from_one_parser() {
    let cases: &[(&str, PrimitiveBuildValue)] = &[
        ("analytics=true", PrimitiveBuildValue::Bool(true)),
        ("channel=false", PrimitiveBuildValue::Bool(false)),
        ("separator=':'", PrimitiveBuildValue::Char(':')),
        (
            "label=\"true\"",
            PrimitiveBuildValue::String(String::from("true")),
        ),
        (
            "build_number=\"42\"",
            PrimitiveBuildValue::String(String::from("42")),
        ),
        (
            "channel=alpha",
            PrimitiveBuildValue::String(String::from("alpha")),
        ),
        (
            "api_url=https://example.com",
            PrimitiveBuildValue::String(String::from("https://example.com")),
        ),
        (
            "version=1.2.3",
            PrimitiveBuildValue::String(String::from("1.2.3")),
        ),
        ("empty=", PrimitiveBuildValue::String(String::from(""))),
        (
            "optional=none",
            PrimitiveBuildValue::String(String::from("none")),
        ),
        (
            "plus_one=+1",
            PrimitiveBuildValue::String(String::from("+1")),
        ),
    ];

    for command_name in ["build", "check", "dev"] {
        for (argument, expected_value) in cases {
            let command = get_command(&args(&[command_name, ".", "--input", argument]))
                .unwrap_or_else(|error| panic!("{command_name} should parse {argument}: {error}"));
            let (name, _) = single_input(argument);
            let inputs = input_set(&command);
            let entry = inputs
                .get(&name)
                .unwrap_or_else(|| panic!("{command_name} should retain {argument}"));
            assert_eq!(
                entry.value(),
                &BuildConfigInputValue::Typed(expected_value.clone()),
                "{command_name} {argument}"
            );
            assert_eq!(
                entry.location(),
                &BuildConfigValueLocation::Command(BuildCommandLocation::new(2)),
                "{command_name} {argument} should carry its --input argument position"
            );
        }
    }
}

/// Every command rejects the same quote-leading suffixes, including trivia that the ordinary
/// lexer omits from its token stream. Terminal whitespace is intentionally rejected as well.
#[test]
fn every_command_rejects_incomplete_quoted_input_suffixes() {
    let cases = [
        (
            "label=\"alpha\"--ignored",
            "label",
            "not a complete Moth String literal",
        ),
        (
            "label=\"alpha\"\nignored",
            "label",
            "not a complete Moth String literal",
        ),
        (
            "label=\"alpha\" ",
            "label",
            "not a complete Moth String literal",
        ),
        (
            "separator=':'--ignored",
            "separator",
            "not a complete Moth Char literal",
        ),
        (
            "separator=':'\nignored",
            "separator",
            "not a complete Moth Char literal",
        ),
        (
            "separator=':' ",
            "separator",
            "not a complete Moth Char literal",
        ),
    ];

    for command_name in ["build", "check", "dev"] {
        for (argument, name, expected_reason) in cases {
            let command = get_command(&args(&[command_name, ".", "--input", argument]))
                .expect("the argument shape parses; only classification inspects the value");
            let error = command_input_set(command_inputs(&command))
                .expect_err("a quote-leading value with a suffix must be rejected");
            assert!(
                error.contains(&format!("Invalid value for --input '{name}'")),
                "{command_name} {argument:?}: {error}"
            );
            assert!(
                error.contains(expected_reason),
                "{command_name} {argument:?}: {error}"
            );
        }
    }
}

#[test]
fn every_command_splits_at_the_first_equals_and_preserves_the_remainder() {
    let argument = "api_url=https://example.com/?a=1&b=2";
    for command_name in ["build", "check", "dev"] {
        let command = get_command(&args(&[command_name, ".", "--input", argument]))
            .expect("command should parse the URL input");
        let (name, _) = single_input(argument);
        let entry = input_set(&command)
            .get(&name)
            .cloned()
            .expect("URL input should be retained");
        assert_eq!(
            entry.value(),
            &BuildConfigInputValue::Typed(PrimitiveBuildValue::String(String::from(
                "https://example.com/?a=1&b=2",
            ))),
            "{command_name} must keep every later '=' inside the value"
        );
    }
}

#[test]
fn every_command_rejects_duplicate_input_names_deterministically() {
    for command_name in ["build", "check", "dev"] {
        let command = get_command(&args(&[
            command_name,
            ".",
            "--input",
            "retries=4",
            "--input",
            "retries=9",
        ]))
        .expect("the argument shape parses; duplicates are decided when the input set is built");
        let error = command_input_set(command_inputs(&command))
            .expect_err("duplicate input names must be rejected");
        assert!(
            error.contains("Duplicate --input name 'retries'"),
            "{command_name}: {error}"
        );
        assert!(
            error.contains("argument position 2"),
            "{command_name}: the earlier argument position must be reported: {error}"
        );
    }
}

#[test]
fn command_input_names_must_be_lower_snake_case() {
    for command_name in ["build", "check", "dev"] {
        for bad_name in ["camelCase=1", "2fast=1", "=1", "with-dash=1"] {
            let error = get_command(&args(&[command_name, ".", "--input", bad_name]))
                .expect_err("non lower_snake_case names must be rejected");
            assert!(
                error.contains("input names must be lower_snake_case"),
                "{command_name} {bad_name}: {error}"
            );
        }
    }
}

#[test]
fn input_flag_requires_a_value_argument_with_name_equals_separator() {
    for command_name in ["build", "check", "dev"] {
        let missing = get_command(&args(&[command_name, ".", "--input"]))
            .expect_err("a trailing --input must be rejected");
        assert!(
            missing.contains("Missing value for --input"),
            "{command_name}: {missing}"
        );

        let flag_shaped = get_command(&args(&[command_name, ".", "--input", "--release"]))
            .expect_err("a flag-shaped value argument must be rejected");
        assert!(
            flag_shaped.contains("Missing value for --input"),
            "{command_name}: {flag_shaped}"
        );

        let no_separator = get_command(&args(&[command_name, ".", "--input", "retries"]))
            .expect_err("a value without '=' must be rejected");
        assert!(
            no_separator.contains("Expected format is name=value"),
            "{command_name}: {no_separator}"
        );
    }
}

#[test]
fn command_inputs_surface_compiler_owned_literal_rejections() {
    let command = get_command(&args(&["dev", ".", "--input", "label=\"abc"]))
        .expect("the argument shape parses; the value is rejected when it classifies");
    let unterminated = command_input_set(command_inputs(&command))
        .expect_err("an unterminated String quote must be rejected");
    assert!(
        unterminated.contains("Invalid value for --input 'label'"),
        "{unterminated}"
    );
    assert!(
        unterminated.contains("not a complete Moth String literal"),
        "{unterminated}"
    );
    assert!(
        unterminated.contains("Unterminated string literal"),
        "{unterminated}"
    );

    let command = get_command(&args(&["build", ".", "--input", "separator=':'x"]))
        .expect("the argument shape parses; the value is rejected when it classifies");
    let bad_char = command_input_set(command_inputs(&command))
        .expect_err("trailing text after a Char literal must be rejected");
    assert!(
        bad_char.contains("not a complete Moth Char literal"),
        "{bad_char}"
    );
}

#[test]
fn every_command_keeps_numeric_inputs_deferred() {
    for command_name in ["build", "check", "dev"] {
        let command = get_command(&args(&[command_name, ".", "--input", "count=4294967296"]))
            .expect("numeric input should parse before its receiving contract is known");
        assert!(
            matches!(
                retained_value(&command, "count"),
                BuildConfigInputValue::Deferred(_)
            ),
            "{command_name} must not choose a scalar while parsing"
        );
    }
}

#[test]
fn parsed_inputs_stay_on_the_programmatic_command_options() {
    let build = get_command(&args(&[
        "build",
        ".",
        "--input",
        "retries=4",
        "--input",
        "label=\"42\"",
    ]))
    .expect("build should parse repeated inputs");
    let check = get_command(&args(&["check", ".", "--input", "retries=4"]))
        .expect("check should parse its input");
    let dev = get_command(&args(&["dev", ".", "--input", "retries=4"]))
        .expect("dev should parse its input");

    assert_eq!(command_inputs(&build).len(), 2);
    assert_eq!(command_inputs(&check).len(), 1);
    assert_eq!(command_inputs(&dev).len(), 1);

    let (name, value) = single_input("retries=4");
    assert_eq!(
        input_set(&dev)
            .get(&name)
            .expect("dev retains retries")
            .value(),
        &value
    );
}

#[test]
fn unknown_input_names_remain_in_the_set_for_later_contract_validation() {
    // No project config or source contract exists in this test: the parser must accept and
    // retain the typed value under its unknown name so the later selected-contract phases
    // decide whether it matches a declaration.
    let argument = "totally_unknown_name=alpha";
    let command = get_command(&args(&["build", ".", "--input", argument]))
        .expect("unknown input names parse and stay in the set");
    let (name, value) = single_input(argument);
    let entry = input_set(&command)
        .get(&name)
        .cloned()
        .expect("unknown name must remain in the set");
    assert_eq!(entry.value(), &value);
}

#[cfg(unix)]
#[test]
fn cli_argument_conversion_skips_program_name_and_reports_non_utf8_position() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let error = convert_cli_args([
        OsString::from_vec(b"moth\xff".to_vec()),
        OsString::from("check"),
        OsString::from_vec(b"bad\xff".to_vec()),
    ])
    .expect_err("non-UTF-8 CLI arguments should be rejected");

    assert!(error.contains("Argument position 1"), "{error}");
    assert!(
        error.contains(&format!("'bad{}'", char::REPLACEMENT_CHARACTER)),
        "{error}"
    );
    assert!(
        error.contains("Moth arguments must be valid UTF-8"),
        "{error}"
    );
}
