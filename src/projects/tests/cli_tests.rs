//! Tests for CLI command parsing and validation.

#[cfg(feature = "timers")]
use super::run_build_command_with_output_plan;
#[cfg(feature = "timers")]
use super::run_build_command_with_output_plan_for_tests;
use super::{
    Command, build_warnings_messages, compact_whitespace, get_command, help_build_flag_entries,
    integration_run_status, is_standalone_version_request, run_build_command,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, FileKind, OutputFile, Project};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{BuilderKind, CleanupPolicy, OutputOwner};
use crate::compiler_frontend::Flag;
#[cfg(feature = "boracle")]
use crate::compiler_frontend::analysis::borrow_checker::{
    BoracleDump, BoracleExperiment, BoracleReferenceRuleSet, BoracleRuleSelection,
};
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigInputSet, BuildConfigValueLocation, BuildInputName,
    PrimitiveBuildValue,
};
#[cfg(feature = "timers")]
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, RuleDiagnosticKind,
};
use crate::compiler_frontend::folded_value::FiniteFloat;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_tests::integration_test_runner::{
    BackendId, IntegrationRunSummary, TestRunnerOptions,
};
use crate::compiler_tests::test_fs::assert_path_missing;
use crate::projects::check::{CheckOptions, run_check};
use crate::projects::command_status::CommandStatus;
use crate::projects::dev_server::DevServerOptions;
use crate::projects::html_project::new_html_project::NewHtmlProjectOptions;
use crate::projects::settings::Config;
#[cfg(feature = "timers")]
use crate::timing::start_benchmark_collection;
#[cfg(feature = "boracle")]
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
#[cfg(feature = "timers")]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "timers")]
use std::time::Duration;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn write_cli_config_input_project() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("should create temporary project");
    let root = temp.path();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    dev_output = \"preview\",\n    release_output = \"release\",\n|\n",
    )
    .expect("should write project config");
    fs::write(
        source_root.join("@page.moth"),
        "enabled #Config of Bool\nresult ~= 0\nif enabled:\n    result = 1\n;\n",
    )
    .expect("should write config-input source");
    temp
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
            inputs: BuildConfigInputSet::new(),
        }
    );
}

#[test]
fn build_command_writes_the_validated_directory_output_plan() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    dev_output = \"preview\",\n    release_output = \"release\",\n|\n",
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
        &BuildConfigInputSet::new(),
    );
    assert_eq!(status, CommandStatus::Success);
    assert!(root.join("preview/index.html").exists());
    assert_path_missing(&root.join("dev/index.html"));
}

#[test]
fn build_command_executes_with_typed_config_input() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = write_cli_config_input_project();
    let root = temp.path().to_path_buf();
    let root_text = root
        .to_str()
        .expect("temporary project path should be valid UTF-8");
    let command = get_command(&args(&["build", root_text, "--input", "enabled=true"]))
        .expect("build command with a typed input should parse");
    let Command::Build {
        path,
        flags,
        inputs,
    } = command
    else {
        panic!("build command expected");
    };
    let enabled = BuildInputName::new("enabled").expect("test input name should validate");
    assert_eq!(
        inputs
            .get(&enabled)
            .expect("build should retain enabled")
            .value(),
        &PrimitiveBuildValue::Bool(true),
        "build should retain the typed Bool value"
    );

    let status = run_build_command(&path, &flags, &inputs);

    assert_eq!(status, CommandStatus::Success);
    assert!(
        root.join("preview/index.html").exists(),
        "successful build should emit its configured output"
    );
}

#[test]
fn check_command_executes_with_typed_config_input() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = write_cli_config_input_project();
    let root = temp.path().to_path_buf();
    let root_text = root
        .to_str()
        .expect("temporary project path should be valid UTF-8");
    let command = get_command(&args(&["check", root_text, "--input", "enabled=true"]))
        .expect("check command with a typed input should parse");
    let Command::Check {
        path,
        terse,
        inputs,
    } = command
    else {
        panic!("check command expected");
    };
    let enabled = BuildInputName::new("enabled").expect("test input name should validate");
    assert_eq!(
        inputs
            .get(&enabled)
            .expect("check should retain enabled")
            .value(),
        &PrimitiveBuildValue::Bool(true),
        "check should retain the typed Bool value"
    );

    let status = run_check(&path, CheckOptions { terse, inputs });

    assert_eq!(status, CommandStatus::Success);
    assert!(
        !root.join("preview").exists(),
        "check should validate the source without writing build output"
    );
}

/// An output-plan failure still reaches the command's single timing finish.
#[cfg(feature = "timers")]
#[test]
fn failed_output_plan_records_the_build_command_total() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

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
        &BuildConfigInputSet::new(),
        |_| Err(CompilerError::compiler_error("forced output-plan failure")),
    );
    let snapshot = timing_session.finish();

    assert_eq!(status, CommandStatus::Failure);
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|observation| {
                observation.metric.descriptor().stable_name == "command.build.total"
            })
            .expect("the command total must retain a dense row")
            .samples,
        1,
        "the failed output-plan path must finish the command total before the session drains"
    );
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|observation| {
                observation.metric.descriptor().stable_name == "build.output.total"
            })
            .expect("the output total must retain a dense row")
            .samples,
        1,
        "the failed output-plan path must finish the output segment before the session drains"
    );
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
            inputs: BuildConfigInputSet::new(),
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
                inputs: BuildConfigInputSet::new(),
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
                inputs: BuildConfigInputSet::new(),
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
            inputs: BuildConfigInputSet::new(),
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
            inputs: BuildConfigInputSet::new(),
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
            inputs: BuildConfigInputSet::new(),
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
#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_parses_typed_dump_and_repeated_experiments() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error =
        get_command(&args(&["boracle"])).expect_err("Boracle command should require a source path");
    assert!(error.contains("requires one source path"));
}

#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_rejects_unknown_experiment() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["boracle", "main.moth", "--experiment", "unknown"]))
        .expect_err("unknown Boracle experiment should fail");
    assert!(error.contains("Unknown Boracle experiment"));
}

#[cfg(feature = "boracle")]
#[test]
fn boracle_cli_rejects_comma_separated_experiments() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let error = get_command(&args(&["boracle", "main.moth", "--dump", "unknown"]))
        .expect_err("unknown Boracle dump should fail");
    assert!(error.contains("Invalid value for --dump"));
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
            inputs: BuildConfigInputSet::new(),
        }
    );

    let wasm = get_command(&args(&["build", "--html-wasm"])).expect("html-wasm flag should parse");
    assert_eq!(
        wasm,
        Command::Build {
            path: String::new(),
            flags: vec![Flag::HtmlWasm],
            inputs: BuildConfigInputSet::new(),
        }
    );

    let both = get_command(&args(&["build", "--release", "--html-wasm"]))
        .expect("both flags should parse");
    assert_eq!(
        both,
        Command::Build {
            path: String::new(),
            flags: vec![Flag::Release, Flag::HtmlWasm],
            inputs: BuildConfigInputSet::new(),
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
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings,
        string_table: StringTable::new(),
        source_database: None,
        warning_source_contexts: Vec::new(),
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

/// Baseline test: build command records command.build.total with exactly one sample.
#[cfg(feature = "timers")]
#[test]
fn successful_build_records_command_build_total() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create temporary project root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config file");
    fs::write(source_root.join("@page.moth"), "value = 1\n").expect("should write source file");

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let status = run_build_command(
        root.to_str().expect("temporary path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );
    let snapshot = timing_session.finish();

    assert_eq!(status, CommandStatus::Success);
    let command_total = snapshot
        .timings
        .iter()
        .find(|observation| observation.metric.descriptor().stable_name == "command.build.total")
        .expect("command.build.total must be recorded");
    assert_eq!(command_total.samples, 1);
}

/// Boundary regression: a scripted build duration is recorded before rendering and
/// renderer work cannot change the command total.
///
/// WHAT: proves execute/classify -> capture scripted duration -> render ordering
///       by injecting a renderer callback that performs observable work after capture.
/// WHY:  the original bug had presentation between two timing boundaries. This test
///       pins the corrected boundary so renderer work stays outside the command total.
#[cfg(feature = "timers")]
#[test]
fn build_command_total_excludes_renderer_work() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(source_root.join("@page.moth"), "value = 1\n").expect("should write source");

    let scripted_duration = Duration::from_millis(42);
    let renderer_calls = AtomicUsize::new(0);

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let (status, _) = run_build_command_with_output_plan_for_tests(
        root.to_str().expect("temporary path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        super::create_build_output_plan,
        scripted_duration,
        |_outcome, duration| {
            // The renderer receiving the captured duration is the ordering evidence: capture
            // already happened. Sleeping here would add wall-clock time to the test without
            // strengthening that, because the recorded total is the scripted value, not a
            // measurement of this callback.
            assert_eq!(duration, scripted_duration);
            renderer_calls.fetch_add(1, Ordering::SeqCst);
        },
    );
    let snapshot = timing_session.finish();

    assert_eq!(status, CommandStatus::Success);
    assert_eq!(
        renderer_calls.load(Ordering::SeqCst),
        1,
        "renderer must run after capture"
    );

    let command_total = snapshot
        .timings
        .iter()
        .find(|observation| observation.metric.descriptor().stable_name == "command.build.total")
        .expect("command.build.total must be recorded");

    // The structured total must equal the scripted duration exactly, proving
    // renderer work did not enter the captured boundary.
    assert_eq!(command_total.total, scripted_duration);
    assert_eq!(command_total.samples, 1, "exactly one command-total sample");
}

/// A successful build reports the artifacts the writer emitted, not the artifacts the backend
/// planned.
///
/// WHAT: replaces the built project's outputs with one emitted page beside one `NotBuilt` entry
///       and asserts the rendered count is the emitted one.
/// WHY: `NotBuilt` exists so a backend can declare an artifact it deliberately does not write.
///      Counting the planned list would tell the user the compiler produced a file that is not
///      on disk.
#[cfg(feature = "timers")]
#[test]
fn build_success_counts_emitted_artifacts_not_planned_ones() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(source_root.join("@page.moth"), "value = 1\n").expect("should write source");

    let reported_count = std::cell::Cell::new(None);
    let (status, _) = run_build_command_with_output_plan_for_tests(
        root.to_str().expect("temporary path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        |build_result| {
            build_result.project.output_files = vec![
                OutputFile::new(
                    PathBuf::from("index.html"),
                    FileKind::Html(String::from("<html></html>")),
                ),
                OutputFile::new(PathBuf::from("unbuilt.js"), FileKind::NotBuilt),
            ];
            super::create_build_output_plan(build_result)
        },
        Duration::from_millis(11),
        |outcome, _| {
            if let super::BuildCommandOutcome::Success {
                output_file_count, ..
            } = outcome
            {
                reported_count.set(Some(*output_file_count));
            }
        },
    );

    assert_eq!(status, CommandStatus::Success);
    assert_eq!(
        reported_count.get(),
        Some(1),
        "the planned list holds two artifacts, but only the page was emitted"
    );
    assert_path_missing(&root.join("dev/unbuilt.js"));
}

/// Boundary regression: a failed output write still records one command total and
/// one output total, and rendering happens after capture.
#[cfg(feature = "timers")]
#[test]
fn failed_output_write_records_build_command_total() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let status = run_build_command_with_output_plan(
        entry_file
            .to_str()
            .expect("temporary path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        |build_result| {
            // Replace the valid output file with an escaping path so the plan
            // succeeds but the filesystem write fails validation.
            build_result.project.output_files = vec![OutputFile::new(
                PathBuf::from("../escape.html"),
                FileKind::Html(String::from("<html></html>")),
            )];
            super::create_build_output_plan(build_result)
        },
    );
    let snapshot = timing_session.finish();

    assert_eq!(status, CommandStatus::Failure);
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|observation| observation.metric.descriptor().stable_name == "command.build.total")
            .expect("the command total must retain a dense row")
            .samples,
        1,
        "the failed output-write path must finish the command total before rendering"
    );
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|observation| observation.metric.descriptor().stable_name == "build.output.total")
            .expect("the output total must retain a dense row")
            .samples,
        1,
        "the failed output-write path must finish the output segment before the session drains"
    );
}

// -------------------------
//  Shared `--input name=value` command parser
// -------------------------

fn command_inputs(command: &Command) -> &BuildConfigInputSet {
    match command {
        Command::Build { inputs, .. } | Command::Check { inputs, .. } => inputs,
        Command::Dev { options, .. } => &options.inputs,
        _ => panic!("command variant does not carry build-config inputs"),
    }
}

fn single_input(argument: &str) -> (BuildInputName, PrimitiveBuildValue) {
    let (name_text, value_text) = argument.split_once('=').expect("test argument has '='");
    (
        BuildInputName::new(name_text).expect("test input name should validate"),
        PrimitiveBuildValue::from_command_text(value_text).expect("test input value should infer"),
    )
}

fn cli_float(value: f64) -> PrimitiveBuildValue {
    PrimitiveBuildValue::Float(FiniteFloat::new(value).expect("test float should be finite"))
}

/// Every command variant shares one parser and one typed carrier: the same argument text
/// infers the same value with the same command location on build, check and dev, and no
/// project config, source contract or build-system state exists in this test.
#[test]
fn every_command_infers_primitive_values_immediately_from_one_parser() {
    let cases: &[(&str, PrimitiveBuildValue)] = &[
        ("analytics=true", PrimitiveBuildValue::Bool(true)),
        ("channel=false", PrimitiveBuildValue::Bool(false)),
        ("retries=4", PrimitiveBuildValue::Int(4)),
        ("offset=-3", PrimitiveBuildValue::Int(-3)),
        ("ratio=0.75", cli_float(0.75)),
        ("scale=1e-2", cli_float(1e-2)),
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
            let inputs = command_inputs(&command);
            let (name, _) = single_input(argument);
            let entry = inputs
                .get(&name)
                .unwrap_or_else(|| panic!("{command_name} should retain {argument}"));
            assert_eq!(entry.value(), expected_value, "{command_name} {argument}");
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
            let error = get_command(&args(&[command_name, ".", "--input", argument]))
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
        let entry = command_inputs(&command)
            .get(&name)
            .expect("URL input should be retained");
        assert_eq!(
            entry.value(),
            &PrimitiveBuildValue::String(String::from("https://example.com/?a=1&b=2")),
            "{command_name} must keep every later '=' inside the value"
        );
    }
}

#[test]
fn every_command_rejects_duplicate_input_names_deterministically() {
    for command_name in ["build", "check", "dev"] {
        let error = get_command(&args(&[
            command_name,
            ".",
            "--input",
            "retries=4",
            "--input",
            "retries=9",
        ]))
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
fn command_inputs_surface_the_compiler_owned_rejections() {
    let overflow = get_command(&args(&["build", ".", "--input", "retries=2147483648"]))
        .expect_err("Int overflow must be a diagnostic, not String fallback");
    assert!(
        overflow.contains("whole-number value '2147483648' is outside the Int range"),
        "{overflow}"
    );

    let non_finite = get_command(&args(&["check", ".", "--input", "ratio=1e400"]))
        .expect_err("non-finite exponent values must reject");
    assert!(
        non_finite.contains("float value '1e400' is not finite"),
        "{non_finite}"
    );

    let unterminated = get_command(&args(&["dev", ".", "--input", "label=\"abc"]))
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

    let bad_char = get_command(&args(&["build", ".", "--input", "separator=':'x"]))
        .expect_err("trailing text after a Char literal must be rejected");
    assert!(
        bad_char.contains("not a complete Moth Char literal"),
        "{bad_char}"
    );
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

    let Command::Dev { options, .. } = dev else {
        panic!("dev command expected");
    };
    assert_eq!(options.inputs.len(), 1);
    let (name, value) = single_input("retries=4");
    assert_eq!(
        options
            .inputs
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
    let entry = command_inputs(&command)
        .get(&name)
        .expect("unknown name must remain in the set");
    assert_eq!(entry.value(), &value);
}
