//! Command-line entrypoints for the Moth toolchain.
//!
//! This module parses CLI commands and dispatches them into build, dev-server, scaffolding, and
//! compiler test workflows.

use crate::command_status::{CommandStatus, emit_benchmark_status};
use crate::scaffold::{self, NewHtmlProjectOptions};
#[cfg(feature = "boracle")]
use moth::projects::boracle::run_boracle;
use moth::projects::build_command::{
    BuildCommandOutcome, BuildCommandReport, run_build_command as run_build_service,
};
use moth::projects::check::{CheckCommandReport, run_check_command};
use moth::projects::dev_server::{self, DevServerOptions};
use moth::projects::integration_tests::{
    BackendId, IntegrationRunSummary, TestRunnerOptions, run_all_test_cases,
};
#[cfg(feature = "boracle")]
use moth::projects::{BoracleDump, BoracleExperiment, BoracleRuleSelection};
use moth::projects::{
    BuildCommandLocation, BuildConfigInputDuplicate, BuildConfigInputEntry, BuildConfigInputSet,
    BuildConfigInputValue, BuildConfigValueLocation, BuildInputName, BuildInputValueError, Flag,
    HtmlProjectBuilder, ProjectBuilder, ProjectCommandCompletion, ProjectCommandStatus,
    print_compiler_messages, print_terse_compiler_messages,
};
use saying::say;
use std::{env, process};

/// Build-profile flags accepted by both `build` and `dev`.
const BUILD_FLAGS: &[&str] = &["--release", "--html-wasm"];

/// The error text used when an `--input` flag carries no `name=value` argument.
const MISSING_COMMAND_INPUT_VALUE: &str = "Missing value for --input.";

/// One `--input name=value` argument whose value is not yet classified.
///
/// WHAT: the validated input name, the raw authored value text and the zero-based position of the
///       `--input` argument that carried them.
/// WHY:  a command value's primitive type depends on the config contract that receives it, and
///       that contract is only known once the project compiles. The argument shape and name are
///       still validated and stored while parsing, so a malformed argument is rejected before any
///       compilation work begins. The raw value keeps the authored spelling that classification
///       and diagnostics report.
#[derive(Debug, PartialEq, Eq)]
struct CommandInputArgument {
    name: BuildInputName,
    value: String,
    /// Zero-based position of the carrying `--input` argument, reported by duplicate diagnostics.
    argument_index: usize,
}

/// Build the typed input set one command compiles with from every parsed `--input` argument.
///
/// WHAT: the one classification path build, check and dev share. Each value is classified from its
///       authored text alone, entries keep their argument order, and a repeated name is rejected
///       deterministically while the earlier entry keeps its place.
/// WHY:  a command number has no intrinsic width, so no builder chooses one here. The receiving
///       contract materialises it at its exact scalar.
fn command_input_set(arguments: &[CommandInputArgument]) -> Result<BuildConfigInputSet, String> {
    let mut inputs = BuildConfigInputSet::new();

    for argument in arguments {
        let value =
            BuildConfigInputValue::from_command_text(&argument.value).map_err(
                |error| match error {
                    BuildInputValueError::Infrastructure(error) => {
                        format!(
                            "Failed to process --input '{}': {}.",
                            argument.name.as_str(),
                            error.msg
                        )
                    }
                    error => format!(
                        "Invalid value for --input '{}': {error}.",
                        argument.name.as_str()
                    ),
                },
            )?;

        let entry = BuildConfigInputEntry::new(
            argument.name.clone(),
            value,
            BuildConfigValueLocation::Command(BuildCommandLocation::new(argument.argument_index)),
        );

        if let Err(duplicate) = inputs.insert(entry) {
            return Err(command_input_duplicate_message(&duplicate));
        }
    }

    Ok(inputs)
}

/// Parse one shared `--input name=value` value argument and retain it for classification.
///
/// WHAT: the one input parser build, check and dev share. It rejects a missing or flag-shaped
///       value argument, splits the argument at the first `=` so every later `=` stays inside
///       the value, and validates the lower_snake_case name. The retained argument carries the
///       raw value text and the zero-based position of the `--input` argument.
/// WHY:  all three commands must share one parser and one typed carrier — no command owns a
///       second conversion or defaulting path, and unknown input names stay in the set for the
///       later selected-contract validation phases. The value is classified by
///       [`command_input_set`], and a numeric value materialises only at the contract that
///       receives it.
fn parse_command_input_argument(
    value_argument: Option<&String>,
    argument_index: usize,
    arguments: &mut Vec<CommandInputArgument>,
) -> Result<(), String> {
    let Some(value_argument) = value_argument.filter(|argument| !argument.starts_with("--")) else {
        return Err(String::from(MISSING_COMMAND_INPUT_VALUE));
    };

    let Some((name_text, value_text)) = value_argument.split_once('=') else {
        return Err(format!(
            "Invalid value for --input: '{value_argument}'. Expected format is name=value."
        ));
    };

    let name = BuildInputName::new(name_text).map_err(|_| {
        format!("Invalid --input name '{name_text}': input names must be lower_snake_case.")
    })?;

    arguments.push(CommandInputArgument {
        name,
        value: value_text.to_owned(),
        argument_index,
    });

    Ok(())
}

/// Render the deterministic duplicate-input rejection for the command error surface.
///
/// Command inputs have no source span, so the carrier's retained command location — the
/// earlier `--input` argument position — is the provenance a duplicate diagnostic reports.
fn command_input_duplicate_message(duplicate: &BuildConfigInputDuplicate) -> String {
    let name = duplicate.rejected().name().as_str();
    match duplicate.existing_location() {
        BuildConfigValueLocation::Command(location) => format!(
            "Duplicate --input name '{name}': an earlier value was given at argument position {}.",
            location.argument_index()
        ),
        BuildConfigValueLocation::Source(_) => {
            format!("Duplicate --input name '{name}': an earlier value was already given.")
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    NewHTMLProject(NewHtmlProjectOptions), // Creates a new HTML project template

    Build {
        path: String,
        flags: Vec<Flag>,
        inputs: Vec<CommandInputArgument>,
    }, // Builds a file or project

    Check {
        path: String,
        terse: bool,
        inputs: Vec<CommandInputArgument>,
    }, // Runs frontend-only compilation without writing artefacts

    #[cfg(feature = "boracle")]
    Boracle {
        path: String,
        dump: BoracleDump,
        rule_selection: BoracleRuleSelection,
    }, // Runs the internal, unstable reference-solver source service

    // Runs a hot reloading dev server that can be accessed in the browser
    // Will only support HTML projects for now
    Dev {
        path: String,
        options: DevServerOptions,
        flags: Vec<Flag>,
        inputs: Vec<CommandInputArgument>,
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
                    match scaffold::create_html_project_template(options) {
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

                Command::Build {
                    path,
                    flags,
                    inputs,
                } => {
                    let project_builder = ProjectBuilder::new(HtmlProjectBuilder::new());
                    match command_input_set(&inputs) {
                        Ok(inputs) => run_build_command(&project_builder, &path, &flags, &inputs),
                        Err(message) => {
                            say!(message);
                            print_help();
                            CommandStatus::Failure
                        }
                    }
                }

                Command::Check {
                    path,
                    terse,
                    inputs,
                } => {
                    let project_builder = ProjectBuilder::new(HtmlProjectBuilder::new());
                    match command_input_set(&inputs) {
                        Ok(inputs) => {
                            let completion =
                                run_check_command(&path, &inputs, &project_builder, |report| {
                                    render_check_outcome(report, terse)
                                });
                            if let Some(counts) = completion.diagnostic_counts {
                                emit_benchmark_status(counts.errors, counts.warnings);
                            }
                            command_status_from_completion(completion)
                        }
                        Err(message) => {
                            say!(message);
                            print_help();
                            CommandStatus::Failure
                        }
                    }
                }

                #[cfg(feature = "boracle")]
                Command::Boracle {
                    path,
                    dump,
                    rule_selection,
                } => match run_boracle(&path, dump, rule_selection) {
                    Ok(report) => {
                        print!("{report}");
                        CommandStatus::Success
                    }
                    Err(messages) => {
                        print_compiler_messages(messages);
                        CommandStatus::Failure
                    }
                },

                Command::Dev {
                    path,
                    options,
                    flags,
                    inputs,
                } => {
                    let project_builder = ProjectBuilder::new(HtmlProjectBuilder::new());
                    match command_input_set(&inputs) {
                        Ok(inputs) => {
                            say!("\nStarting dev server...");
                            let options = options.with_inputs(inputs);
                            match dev_server::run_dev_server(
                                project_builder,
                                &path,
                                &flags,
                                options,
                            ) {
                                Ok(_) => CommandStatus::Success,
                                Err(messages) => {
                                    print_compiler_messages(messages);
                                    CommandStatus::Failure
                                }
                            }
                        }
                        Err(message) => {
                            say!(message);
                            print_help();
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
                                    compact_whitespace(&error.message)
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

fn run_build_command(
    project_builder: &ProjectBuilder,
    path: &str,
    flags: &[Flag],
    build_config_inputs: &BuildConfigInputSet,
) -> CommandStatus {
    let completion = run_build_service(
        project_builder,
        path,
        flags,
        build_config_inputs,
        render_build_outcome,
    );
    if let Some(counts) = completion.diagnostic_counts {
        emit_benchmark_status(counts.errors, counts.warnings);
    }
    command_status_from_completion(completion)
}

fn render_build_outcome(report: BuildCommandReport) {
    match report.outcome {
        BuildCommandOutcome::Success {
            output_file_count,
            warnings,
            ..
        } => {
            print_build_success_message(output_file_count, report.duration);
            if let Some(messages) = warnings {
                print_compiler_messages(messages);
            }
        }
        BuildCommandOutcome::Failure { messages, .. } => print_compiler_messages(messages),
    }
}

fn render_check_outcome(report: CheckCommandReport, terse: bool) {
    if terse {
        print_terse_compiler_messages(report.messages);
        println!(
            "{}",
            format_terse_summary_line(report.duration, report.error_count, report.warning_count)
        );
    } else if report.error_count == 0 && report.warning_count == 0 {
        say!(Dark White "---------------------");
        say!(success_message(report.duration));
        say!(Bold Green "No errors or warnings");
    } else {
        print_compiler_messages(report.messages);
    }
}

fn command_status_from_completion(completion: ProjectCommandCompletion) -> CommandStatus {
    match completion.status {
        ProjectCommandStatus::Success => CommandStatus::Success,
        ProjectCommandStatus::Failure => CommandStatus::Failure,
    }
}

fn format_terse_summary_line(
    duration: std::time::Duration,
    error_count: usize,
    warning_count: usize,
) -> String {
    if error_count == 0 && warning_count == 0 {
        return format!("{}. No errors or warnings.", success_message(duration));
    }

    format!("errors={error_count}, warnings={warning_count}.")
}

fn format_duration(duration: std::time::Duration) -> String {
    format!("{duration:?}")
}

fn success_message(duration: std::time::Duration) -> String {
    format!("Done in {}", format_duration(duration))
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

        #[cfg(feature = "boracle")]
        Some("boracle") => parse_boracle_command(args),

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
    let mut inputs = Vec::new();
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
            "--input" => {
                parse_command_input_argument(args.get(index + 1), index, &mut inputs)?;
                index += 2;
            }
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "Unknown build flag: '{arg}'. Supported build flags are {} and --input name=value.",
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

    Ok(Command::Build {
        path,
        flags,
        inputs,
    })
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

    options.validate().map_err(|error| error.to_string())?;

    Ok(Command::CompilerTests { options })
}

fn parse_check_command(args: &[String]) -> Result<Command, String> {
    let mut path = String::new();
    let mut terse = false;
    let mut inputs = Vec::new();
    let mut index = 1usize;

    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--terse" => {
                terse = true;
                index += 1;
            }
            "--input" => {
                parse_command_input_argument(args.get(index + 1), index, &mut inputs)?;
                index += 2;
            }
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "Unknown check flag: '{arg}'. Supported check flags are --terse and --input name=value."
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

    Ok(Command::Check {
        path,
        terse,
        inputs,
    })
}

#[cfg(feature = "boracle")]
fn parse_boracle_command(args: &[String]) -> Result<Command, String> {
    let mut path = String::new();
    let mut dump = BoracleDump::Problem;
    let mut rule_selection = BoracleRuleSelection::default();
    let mut dump_seen = false;
    let mut index = 1usize;

    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--dump" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --dump."));
                };
                if value.starts_with("--") || value.trim().is_empty() {
                    return Err(String::from("Missing value for --dump."));
                }
                if dump_seen {
                    return Err(String::from("Boracle command accepts --dump at most once."));
                }
                dump = value
                    .parse()
                    .map_err(|error: String| format!("Invalid value for --dump: {error}"))?;
                dump_seen = true;
                index += 2;
            }
            "--experiment" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(String::from("Missing value for --experiment."));
                };
                if value.starts_with("--") || value.trim().is_empty() {
                    return Err(String::from("Missing value for --experiment."));
                }
                let experiment = value
                    .parse::<BoracleExperiment>()
                    .map_err(|error: String| format!("Invalid value for --experiment: {error}"))?;
                rule_selection.experiments.insert(experiment);
                index += 2;
            }
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "Unknown boracle flag: '{arg}'. Supported flags are --dump <problem|origins|relations|precision|loans|last-use|conflicts|witnesses|differential> and --experiment <dead-exclusive-loan>."
                ));
            }
            _ => {
                if path.is_empty() {
                    path = arg.to_owned();
                    index += 1;
                } else {
                    return Err(String::from(
                        "Boracle command accepts at most one path argument.",
                    ));
                }
            }
        }
    }

    if path.trim().is_empty() {
        return Err(String::from("Boracle command requires one source path."));
    }

    rule_selection
        .validate()
        .map_err(|error| format!("Invalid Boracle rule selection: {error}"))?;

    Ok(Command::Boracle {
        path,
        dump,
        rule_selection,
    })
}

fn parse_dev_command(args: &[String]) -> Result<Command, String> {
    let mut path = String::new();
    let mut options = DevServerOptions::default();
    let mut flags = Vec::new();
    let mut inputs = Vec::new();
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
            "--input" => {
                parse_command_input_argument(args.get(index + 1), index, &mut inputs)?;
                index += 2;
            }
            _ if arg.starts_with("--") => {
                return Err(format!(
                    "Unknown dev flag: '{arg}'. Supported dev flags are --host, --port, --poll-interval-ms, --release, --html-wasm, and --input name=value."
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
        inputs,
    })
}

/// Returns the flag entries shown in the help text for build and dev.
fn help_build_flag_entries() -> &'static [&'static str] {
    &[
        "  --release               (selects the release build profile and output folder)",
        "  --html-wasm             (uses the experimental HTML-Wasm backend)",
        "  --input name=value      (repeatable; supplies a typed build configuration value)",
    ]
}
fn print_help() {
    say!(Green Bold "Moth", Reset " is version ", Blue Bold env!("CARGO_PKG_VERSION"));

    say!(Green Bold "\nCommands:");
    say!("  build [path]      - Builds a project");
    say!("  check [path]      - Runs frontend-only diagnostics (no artifacts)");
    #[cfg(feature = "boracle")]
    say!("  boracle <path>    - Internal unstable reference-solver analysis");
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
    say!("  --input name=value      (repeatable; supplies a typed build configuration value)");
    #[cfg(feature = "boracle")]
    {
        say!("\nBoracle command options (internal and unstable):");
        say!(
            "  --dump <section>       (problem, origins, relations, precision, loans, last-use, conflicts, witnesses, differential)"
        );
        say!("  --experiment <name>    (repeatable; dead-exclusive-loan)");
    }
    say!("\nNew command options:");
    say!("  --force                (allows replacing existing scaffold files)");
    say!("\nDev command options:");
    say!("  --host <host>            (default: 127.0.0.1)");
    say!("  --port <port>            (default: 6342)");
    say!("  --poll-interval-ms <ms>  (default: 300)");
}

fn print_build_success_message(output_file_count: usize, duration: std::time::Duration) {
    let file_label = if output_file_count == 1 {
        "output file"
    } else {
        "output files"
    };
    say!(
        "\nBuilt ",
        Blue output_file_count,
        Reset " ", file_label, " successfully in: ",
        Green Bold #duration,
    );
}

fn compact_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tests/cli_tests.rs"]
mod tests;
