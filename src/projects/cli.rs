//! Command-line entrypoints for the Moth toolchain.
//!
//! This module parses CLI commands and dispatches them into build, dev-server, scaffolding, and
//! compiler test workflows.

use crate::build_system::build;
use crate::build_system::build::BuildResult;
use crate::build_system::output::{
    OutputPlan, SingleFileOutputPlan, WriteMode, WriteOptions, write_project_outputs,
};
use crate::command_timing_finish;
use crate::command_timing_scope;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::display_messages::{print_compiler_messages, print_formatted_error};
use crate::compiler_tests::integration_test_runner::{
    BackendId, IntegrationRunSummary, TestRunnerOptions, run_all_test_cases,
};
use crate::projects::check::{self, CheckOptions};
use crate::projects::command_status::{
    CommandStatus, benchmark_diagnostic_counts, emit_benchmark_status,
};
use crate::projects::dev_server::{self, DevServerOptions};
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::html_project::new_html_project::NewHtmlProjectOptions;
use crate::timing_scope;
use saying::say;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{env, process};

/// Build-profile flags accepted by both `build` and `dev`.
const BUILD_FLAGS: &[&str] = &["--release", "--html-wasm"];

#[derive(Debug, PartialEq, Eq)]
enum Command {
    NewHTMLProject(NewHtmlProjectOptions), // Creates a new HTML project template

    Build {
        path: String,
        flags: Vec<Flag>,
    }, // Builds a file or project

    Check {
        path: String,
        terse: bool,
    }, // Runs frontend-only compilation without writing artefacts

    // Runs a hot reloading dev server that can be accessed in the browser
    // Will only support HTML projects for now
    Dev {
        path: String,
        options: DevServerOptions,
        flags: Vec<Flag>,
    },

    Help,
    CompilerTests {
        options: TestRunnerOptions,
    }, // Runs or lists compiler integration tests with composable selection filters
}

pub fn start_cli() -> process::ExitCode {
    let compiler_args: Vec<String> = env::args().collect();
    let cli_args = &compiler_args[1..];

    let status = if cli_args.is_empty() {
        print_help();
        CommandStatus::Success
    } else if cli_args[0].starts_with("--") || cli_args[0].starts_with('-') {
        if is_standalone_version_request(cli_args) {
            println!("moth {}", env!("CARGO_PKG_VERSION"));
            CommandStatus::Success
        } else {
            say!(
                "Invalid standalone flag input: '{}'. Only --version, -v, and -V can be used without a command.",
                cli_args.join(" ")
            );
            print_help();
            CommandStatus::Failure
        }
    } else {
        match get_command(cli_args) {
            Ok(command) => match command {
                Command::Help => {
                    print_help();
                    CommandStatus::Success
                }

                Command::NewHTMLProject(options) => {
                    match crate::projects::html_project::new_html_project::create_html_project_template(
                        options,
                    ) {
                        Ok(_) => CommandStatus::Success,
                        Err(e) if e == "Cancelled project creation." => {
                            println!("{e}");
                            CommandStatus::Success
                        }
                        Err(e) => {
                            println!("{e}");
                            CommandStatus::Failure
                        }
                    }
                }

                Command::Build { path, flags } => run_build_command(&path, &flags),

                Command::Check { path, terse } => {
                    check::run_check(&path, CheckOptions { terse })
                }

                Command::Dev {
                    path,
                    options,
                    flags,
                } => {
                    say!("\nStarting dev server...");
                    let project_builder =
                        build::ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
                    match dev_server::run_dev_server(project_builder, &path, &flags, options) {
                        Ok(_) => CommandStatus::Success,
                        Err(messages) => {
                            print_compiler_messages(messages);
                            CommandStatus::Failure
                        }
                    }
                }

                Command::CompilerTests { options } => {
                    let terse = options.terse;
                    match run_all_test_cases(options) {
                        Ok(summary) => integration_run_status(summary),
                        Err(error) => {
                            if terse {
                                println!(
                                    "Tests failed to run: {}",
                                    compact_whitespace(&error)
                                );
                            } else {
                                say!(Red "Failed to run integration tests:");
                                println!("  {error}");
                            }
                            CommandStatus::Failure
                        }
                    }
                }
            },
            Err(e) => {
                say!(e);
                print_help();
                CommandStatus::Failure
            }
        }
    };

    status.into()
}

fn run_build_command(path: &str, flags: &[Flag]) -> CommandStatus {
    run_build_command_with_output_plan(path, flags, create_build_output_plan)
}

/// Run a build command with one command-total lifecycle and one output-plan decision.
///
/// The injected plan builder keeps the command's terminal timing point shared
/// between ordinary output planning and focused failure-path tests.
fn run_build_command_with_output_plan(
    path: &str,
    flags: &[Flag],
    output_plan_builder: impl FnOnce(&mut BuildResult) -> Result<OutputPlan, CompilerError>,
) -> CommandStatus {
    command_timing_scope!(timing_session, crate::timing::TimingCommandKind::Build);
    let start = Instant::now();
    timing_scope!(timing_guard_command_build_total, "command.build.total");
    let project_builder = build::ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let (status, diagnostic_counts) = match build::build_project(&project_builder, path, flags) {
        Ok(mut build_result) => {
            // Output planning and filesystem emission form one build-pipeline
            // segment. The nested writer records its own `output.write.total`
            // evidence without becoming a second additive command child.
            timing_scope!(timing_guard_build_output_total, "build.output.total");
            match output_plan_builder(&mut build_result) {
                Ok(output_plan) => match write_project_outputs(
                    &build_result.project,
                    &WriteOptions {
                        output_plan,
                        write_mode: WriteMode::AlwaysWrite,
                    },
                    &build_result.string_table,
                ) {
                    Ok(()) => {
                        let duration = start.elapsed();
                        let warning_count = build_result.warnings.len();
                        print_build_message(build_result, duration);
                        (CommandStatus::Success, Some((0, warning_count)))
                    }
                    Err(mut messages) => {
                        messages.extend_diagnostics(build_result.warnings);
                        print_compiler_messages(messages);
                        (CommandStatus::Failure, None)
                    }
                },
                Err(error) => {
                    print_formatted_error(error, &build_result.string_table);
                    (CommandStatus::Failure, None)
                }
            }
        }
        Err(messages) => {
            let diagnostic_counts = benchmark_diagnostic_counts(&messages);
            print_compiler_messages(messages);
            (CommandStatus::Failure, diagnostic_counts)
        }
    };
    #[cfg(feature = "timers")]
    timing_guard_command_build_total.finish();
    command_timing_finish!(timing_session, matches!(status, CommandStatus::Success));
    if let Some((error_count, warning_count)) = diagnostic_counts {
        emit_benchmark_status(error_count, warning_count);
    }
    status
}

/// Choose the output plan after a successful backend build.
///
/// Directory plans are validated during bootstrap. A single-file build needs
/// its synthetic project directory only after backend output exists.
fn create_build_output_plan(build_result: &mut BuildResult) -> Result<OutputPlan, CompilerError> {
    if let Some(plan) = build_result.directory_output_plan.as_ref() {
        return Ok(OutputPlan::Directory(plan.clone()));
    }

    let output_root = env::current_dir().map_err(|error| {
        CompilerError::compiler_error(format!(
            "Could not resolve current directory for build outputs: {error}"
        ))
    })?;
    let project_root = single_file_project_entry_dir(&build_result.config.entry_dir)?;

    Ok(OutputPlan::SingleFile(SingleFileOutputPlan {
        output_root,
        project_root: Some(project_root),
        owner: build_result.output_owner,
        setting_location: SourceLocation::from_path(
            &build_result.config.entry_dir,
            &mut build_result.string_table,
        ),
    }))
}

/// Resolve the containing directory used by single-file output cleanup safety checks.
///
/// WHAT: maps a synthetic single-file entry to the directory that owns it.
/// WHY: output-root validation compares against a directory context, not the source file path.
fn single_file_project_entry_dir(entry_path: &Path) -> Result<PathBuf, CompilerError> {
    let Some(parent) = entry_path.parent() else {
        return Err(CompilerError::compiler_error(format!(
            "Could not resolve containing directory for single-file build entry '{}'",
            entry_path.display()
        )));
    };

    if parent.as_os_str().is_empty() {
        return env::current_dir().map_err(|error| {
            CompilerError::compiler_error(format!(
                "Could not resolve containing directory for single-file build entry '{}': {error}",
                entry_path.display()
            ))
        });
    }

    if parent.is_dir() {
        return Ok(parent.to_path_buf());
    }

    Err(CompilerError::compiler_error(format!(
        "Could not resolve containing directory for single-file build entry '{}': '{}' is not a directory",
        entry_path.display(),
        parent.display()
    )))
}

fn integration_run_status(summary: IntegrationRunSummary) -> CommandStatus {
    if summary.incorrect_results() == 0 {
        CommandStatus::Success
    } else {
        CommandStatus::Failure
    }
}

/// Returns true when every argument is a standalone version spelling (`--version`, `-v`, `-V`).
fn is_standalone_version_request(args: &[String]) -> bool {
    !args.is_empty()
        && args
            .iter()
            .all(|arg| matches!(arg.as_str(), "--version" | "-v" | "-V"))
}

fn get_command(args: &[String]) -> Result<Command, String> {
    let command = args.first().map(String::as_str);

    match command {
        Some("help") => Ok(Command::Help),

        Some("new") => parse_new_command(args),

        Some("build") => parse_build_command(args),

        Some("check") => parse_check_command(args),

        Some("dev") => parse_dev_command(args),

        Some("tests") => parse_tests_command(args),

        Some(other) => Err(format!("Invalid command: '{other}'")),
        None => Err(String::from("Missing command.")),
    }
}

fn parse_new_command(args: &[String]) -> Result<Command, String> {
    match args.get(1).map(String::as_str) {
        Some("html") => {
            let mut raw_path = None;
            let mut force = false;
            let mut index = 2usize;

            while let Some(arg) = args.get(index) {
                match arg.as_str() {
                    "--force" => {
                        force = true;
                        index += 1;
                    }
                    _ if arg.starts_with("--") => {
                        return Err(format!(
                            "Unknown new flag: '{arg}'. Supported flag is --force."
                        ));
                    }
                    _ => {
                        if raw_path.is_none() {
                            raw_path = Some(arg.to_owned());
                            index += 1;
                        } else {
                            return Err(String::from(
                                "New html command accepts at most one path argument.",
                            ));
                        }
                    }
                }
            }

            Ok(Command::NewHTMLProject(NewHtmlProjectOptions { raw_path, force }))
        }
        _ => {
            Err("Invalid project type - currently only 'html' is supported (try 'cargo run -- new html')".to_string())
        }
    }
}

fn parse_build_command(args: &[String]) -> Result<Command, String> {
    let mut path = String::new();
    let mut flags = Vec::new();
    let mut index = 1usize;

    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--release" => {
                flags.push(Flag::Release);
                index += 1;
            }
            "--html-wasm" => {
                flags.push(Flag::HtmlWasm);
                index += 1;
            }
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "Unknown build flag: '{arg}'. Supported build flags are {}.",
                    BUILD_FLAGS.join(", ")
                ));
            }
            _ => {
                if path.is_empty() {
                    path = arg.to_owned();
                    index += 1;
                } else {
                    return Err(String::from(
                        "Build command accepts at most one path argument.",
                    ));
                }
            }
        }
    }

    Ok(Command::Build { path, flags })
}

fn parse_tests_command(args: &[String]) -> Result<Command, String> {
    let mut options = TestRunnerOptions {
        show_warnings: true,
        ..TestRunnerOptions::default()
    };
    let mut index = 1usize;

    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--case" => {
                let Some(case_id) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --case."));
                };
                if case_id.starts_with("--") || case_id.trim().is_empty() {
                    return Err(String::from("Missing value for --case."));
                }
                if options.case_id.is_some() {
                    return Err(String::from(
                        "Tests command accepts at most one --case value.",
                    ));
                }
                options.case_id = Some(case_id.to_owned());
                index += 2;
            }
            "--tag" => {
                let Some(tag) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --tag."));
                };
                if tag.starts_with("--") || tag.trim().is_empty() {
                    return Err(String::from("Missing value for --tag."));
                }
                if options.tag_filters.iter().any(|value| value == tag) {
                    return Err(format!(
                        "Tests command does not accept duplicate --tag value '{tag}'."
                    ));
                }
                options.tag_filters.push(tag.to_owned());
                index += 2;
            }
            "--contract" => {
                let Some(contract) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --contract."));
                };
                if contract.starts_with("--") || contract.trim().is_empty() {
                    return Err(String::from("Missing value for --contract."));
                }
                if options.contract.is_some() {
                    return Err(String::from(
                        "Tests command accepts at most one --contract value.",
                    ));
                }
                options.contract = Some(contract.to_owned());
                index += 2;
            }
            "--backend" => {
                let Some(backend_value) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --backend."));
                };
                if backend_value.starts_with("--") || backend_value.trim().is_empty() {
                    return Err(String::from("Missing value for --backend."));
                }
                if options.backend_filter.is_some() {
                    return Err(String::from(
                        "Tests command accepts at most one --backend value.",
                    ));
                }
                options.backend_filter = Some(
                    BackendId::parse(backend_value)
                        .map_err(|error| format!("Invalid value for --backend: {error}"))?,
                );
                index += 2;
            }
            "--list" => {
                if options.list {
                    return Err(String::from("Tests command accepts --list at most once."));
                }
                options.list = true;
                index += 1;
            }
            "--audit" => {
                if options.audit {
                    return Err(String::from("Tests command accepts --audit at most once."));
                }
                options.audit = true;
                index += 1;
            }
            "--terse" => {
                if options.terse {
                    return Err(String::from("Tests command accepts --terse at most once."));
                }
                options.terse = true;
                index += 1;
            }
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "Unknown tests flag: '{arg}'. Supported tests flags are --case <id>, --tag <tag>, --contract <id>, --backend <html|html_wasm>, --list, --audit, and --terse."
                ));
            }
            _ => {
                return Err(String::from(
                    "Tests command does not accept positional arguments.",
                ));
            }
        }
    }

    options.validate()?;

    Ok(Command::CompilerTests { options })
}

fn parse_check_command(args: &[String]) -> Result<Command, String> {
    let mut path = String::new();
    let mut terse = false;
    let mut index = 1usize;

    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--terse" => {
                terse = true;
                index += 1;
            }
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "Unknown check flag: '{arg}'. Supported check flag is --terse."
                ));
            }
            _ => {
                if path.is_empty() {
                    path = arg.to_owned();
                    index += 1;
                } else {
                    return Err(String::from(
                        "Check command accepts at most one path argument.",
                    ));
                }
            }
        }
    }

    Ok(Command::Check { path, terse })
}

fn parse_dev_command(args: &[String]) -> Result<Command, String> {
    let mut path = String::new();
    let mut options = DevServerOptions::default();
    let mut flags = Vec::new();
    let mut index = 1usize;

    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--host" => {
                let Some(host) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --host"));
                };
                if host.starts_with("--") {
                    return Err(String::from("Missing value for --host"));
                }
                options.host = host.to_owned();
                index += 2;
            }
            "--port" => {
                let Some(port_value) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --port"));
                };
                if port_value.starts_with("--") {
                    return Err(String::from("Missing value for --port"));
                }
                options.port = match port_value.parse::<u16>() {
                    Ok(port) => port,
                    Err(_) => {
                        return Err(format!(
                            "Invalid --port value: '{port_value}'. Port must be a number from 0 to 65535."
                        ));
                    }
                };
                index += 2;
            }
            "--poll-interval-ms" => {
                let Some(interval_value) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --poll-interval-ms"));
                };
                if interval_value.starts_with("--") {
                    return Err(String::from("Missing value for --poll-interval-ms"));
                }
                options.poll_interval_ms = match interval_value.parse::<u64>() {
                    Ok(interval) if interval > 0 => interval,
                    Ok(_) => {
                        return Err(String::from(
                            "Invalid --poll-interval-ms value: '0'. It must be greater than zero.",
                        ));
                    }
                    Err(_) => {
                        return Err(format!(
                            "Invalid --poll-interval-ms value: '{interval_value}'. It must be a positive integer."
                        ));
                    }
                };
                index += 2;
            }
            "--release" => {
                flags.push(Flag::Release);
                index += 1;
            }
            "--html-wasm" => {
                flags.push(Flag::HtmlWasm);
                index += 1;
            }
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "Unknown dev flag: '{arg}'. Supported dev flags are --host, --port, --poll-interval-ms, --release, and --html-wasm."
                ));
            }
            _ => {
                if path.is_empty() {
                    path = arg.to_owned();
                    index += 1;
                } else {
                    return Err(String::from(
                        "Dev command accepts at most one path argument.",
                    ));
                }
            }
        }
    }

    Ok(Command::Dev {
        path,
        options,
        flags,
    })
}

/// Returns the flag entries shown in the help text for build and dev.
fn help_build_flag_entries() -> &'static [&'static str] {
    &[
        "  --release               (selects the release build profile and output folder)",
        "  --html-wasm             (uses the experimental HTML-Wasm backend)",
    ]
}

fn print_help() {
    say!(Green Bold "Moth", Reset " is version ", Blue Bold env!("CARGO_PKG_VERSION"));

    say!(Green Bold "\nCommands:");
    say!("  build [path]      - Builds a project");
    say!("  check [path]      - Runs frontend-only diagnostics (no artifacts)");
    say!("  dev [path]        - Runs the hot reloading dev server");
    say!("  new html [path] [--force] - Creates an HTML project scaffold");
    say!("  tests [options]     - Runs or lists the integration test suite");

    say!(Green Bold "\nBuild and dev flags:");
    for entry in help_build_flag_entries() {
        say!(entry);
    }
    say!("\nTests command options:");
    say!("  --case <id>             (exact canonical case ID)");
    say!("  --tag <tag>             (repeatable; all tags must match)");
    say!("  --contract <id>         (exact contract ID)");
    say!("  --backend <id>          (supported: html, html_wasm)");
    say!("  --list                  (list selected metadata without compiling cases)");
    say!("  --audit                 (write the full suite inventory without compiling cases)");
    say!("  --terse                (compact summary and one-line failure diagnostics)");
    say!("\nCheck command options:");
    say!("  --terse                (compact one-line diagnostics)");
    say!("\nNew command options:");
    say!("  --force                (allows replacing existing scaffold files)");
    say!("\nDev command options:");
    say!("  --host <host>            (default: 127.0.0.1)");
    say!("  --port <port>            (default: 6342)");
    say!("  --poll-interval-ms <ms>  (default: 300)");
}

fn print_build_message(build_result: BuildResult, duration: std::time::Duration) {
    say!(
        "\nBuilt ",
        Blue build_result.project.output_files.len(),
        Reset " files successfully in: ",
        Green Bold #duration,
    );

    if !build_result.warnings.is_empty() {
        let messages =
            CompilerMessages::from_diagnostics(build_result.warnings, build_result.string_table);
        print_compiler_messages(messages);
    }
}

fn compact_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build a `CompilerMessages` container for the warnings produced during a successful build.
///
/// WHAT: centralises the conversion so tests can verify exactly which warnings a successful build
/// would print without relying on stdout capture.
/// WHY: `print_build_message` delegates to `print_compiler_messages`, which writes to the terminal.
/// This helper is the decision boundary for whether any output is produced at all.
#[cfg(test)]
fn build_warnings_messages(build_result: &BuildResult) -> Option<CompilerMessages> {
    if build_result.warnings.is_empty() {
        return None;
    }

    Some(CompilerMessages::from_diagnostics(
        build_result.warnings.clone(),
        build_result.string_table.clone(),
    ))
}

#[cfg(test)]
#[path = "tests/cli_tests.rs"]
mod tests;
