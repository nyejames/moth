//! Dev server runtime implementation.
//!
//! WHAT: validates CLI input, runs the initial dev build, starts the watcher/build loop,
//! and serves HTTP/SSE traffic for hot reload.

use crate::build_system::build::ProjectBuilder;
use crate::build_system::path_validation::check_if_valid_path;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::display_messages::print_diagnostic_reports;
use crate::projects::dev_server::DevServerOptions;
use crate::projects::dev_server::build_loop::{ProjectBuildExecutor, dev_server_error};
use crate::projects::dev_server::state::DevServerState;
use crate::projects::dev_server::watch;
use crate::projects::settings::LANGUAGE_SOURCE_EXTENSION;
use saying::say;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct DevRuntimePaths {
    pub(super) output_dir: PathBuf,
    pub(super) watch_scope: watch::WatchScope,
}

pub fn run_dev_server(
    builder_factory: impl Fn() -> ProjectBuilder + Send + Sync + 'static,
    entry_path: &str,
    flags: &[Flag],
    options: DevServerOptions,
) -> CompilationResult<()> {
    match run_dev_server_with_messages(builder_factory, entry_path, flags, options) {
        Ok(()) => Ok(CompilationOutcome::Success {
            value: (),
            warnings: None,
        }),
        Err(failure) => failure
            .into_diagnostic_reports()
            .map(CompilationOutcome::Diagnosed),
    }
}

fn run_dev_server_with_messages(
    builder_factory: impl Fn() -> ProjectBuilder + Send + Sync + 'static,
    entry_path: &str,
    flags: &[Flag],
    options: DevServerOptions,
) -> Result<(), crate::build_system::build::BootstrapFailure> {
    let entry_target = validate_dev_entry_path(entry_path)?;
    let resolved_paths = resolve_dev_runtime_paths(&entry_target);
    let mut watch_scope = resolved_paths.watch_scope;

    let state = Arc::new(DevServerState::new(resolved_paths.output_dir.clone()));
    let executor = ProjectBuildExecutor::new(builder_factory, options.inputs);

    let mut initial_build_report = crate::projects::dev_server::build_loop::run_single_build_cycle(
        &state,
        &executor,
        &entry_target,
        flags,
    );
    if let Some(updated_watch_scope) = initial_build_report.watch_scope.take() {
        watch_scope = updated_watch_scope;
    }
    if initial_build_report.build_ok {
        say!(
            Green "Initial dev build succeeded. Reload broadcast to ",
            Green initial_build_report.clients_notified,
            Green " clients."
        );
        if let Some(messages) = &initial_build_report.success_messages {
            print_diagnostic_reports(messages);
        }
        for warning in &initial_build_report.output_warnings {
            say!(Yellow warning);
        }
    } else {
        say!(
            Yellow "Initial dev build failed. Reload broadcast to ",
            Yellow initial_build_report.clients_notified,
            Yellow " clients."
        );
    }
    #[cfg(feature = "timers")]
    if let Some(snapshot) = &initial_build_report.timing_snapshot {
        crate::timing::render_command_timing_summary(
            snapshot,
            crate::timing::TimingCommandKind::Dev,
            initial_build_report.build_ok,
        );
    }
    // The startup report is not retained: dropping it here releases any warning report and its
    // identity context before the server runs for its whole lifetime.
    drop(initial_build_report);

    let bind_addr = format!("{}:{}", options.host, options.port);
    let listener = TcpListener::bind(&bind_addr).map_err(|error| {
        dev_server_error(
            &entry_target,
            format!("Failed to start dev server on {bind_addr}: {error}"),
        )
    })?;

    let host_display = if options.host == "127.0.0.1" {
        "localhost"
    } else {
        options.host.as_str()
    };
    say!(Bold "Dev server listening at:");
    say!(
        Green "http://",
        Green host_display,
        Green ":",
        Green options.port
    );

    let watch_state = Arc::clone(&state);
    let watch_entry_file = entry_target.clone();
    let watch_flags = flags.to_vec();
    let poll_interval = Duration::from_millis(options.poll_interval_ms);

    // Watch/rebuild runs independently from request handling so SSE clients do not block rebuilds.
    thread::spawn(move || {
        crate::projects::dev_server::build_loop::run_watch_build_loop(
            watch_state,
            executor,
            watch_entry_file,
            watch_flags,
            watch_scope,
            poll_interval,
        );
    });

    // The server keeps the accept loop simple: each connection is handled on a small worker thread.
    for stream_result in listener.incoming() {
        match stream_result {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(error) =
                        crate::projects::dev_server::http::handle_connection(stream, state)
                    {
                        say!(
                            Yellow "Dev server request handling warning: ",
                            Yellow error.to_string()
                        );
                    }
                });
            }
            Err(error) => {
                say!(
                    Yellow "Dev server connection accept warning: ",
                    Yellow error.to_string()
                );
            }
        }
    }

    Ok(())
}

pub(crate) fn resolve_dev_runtime_paths(entry_target: &Path) -> DevRuntimePaths {
    let project_root = if entry_target.is_dir() {
        entry_target.to_path_buf()
    } else {
        entry_target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let output_dir = project_root.join("dev");
    DevRuntimePaths {
        watch_scope: watch::WatchScope::derive(entry_target, None, &output_dir),
        output_dir,
    }
}

pub(crate) fn validate_dev_entry_path(entry_path: &str) -> Result<PathBuf, InfrastructureFailure> {
    let resolved_path = if entry_path.trim().is_empty() {
        std::env::current_dir().map_err(|error| {
            dev_server_error(
                Path::new("."),
                format!("Failed to resolve current directory: {error}"),
            )
        })?
    } else {
        check_if_valid_path(entry_path)?
    };

    if resolved_path.is_dir() {
        return match resolved_path.canonicalize() {
            Ok(canonical_path) => Ok(canonical_path),
            Err(error) => Err(InfrastructureFailure::filesystem(
                CompilerStage::DevelopmentServer,
                &resolved_path,
                format!("Failed to canonicalize dev entry path: {error}"),
            )),
        };
    }
    if !resolved_path.is_file() {
        return Err(dev_server_error(
            &resolved_path,
            "Dev server entry path must resolve to either a project directory or a .moth file.",
        ));
    }

    let is_moth_file = resolved_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == LANGUAGE_SOURCE_EXTENSION);
    if !is_moth_file {
        return Err(dev_server_error(
            &resolved_path,
            "Dev server currently only supports .moth file entries.",
        ));
    }

    match resolved_path.canonicalize() {
        Ok(canonical_path) => Ok(canonical_path),
        Err(error) => Err(InfrastructureFailure::filesystem(
            CompilerStage::DevelopmentServer,
            &resolved_path,
            format!("Failed to canonicalize dev entry path: {error}"),
        )),
    }
}
